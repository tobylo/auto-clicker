//! The events output builds, checked without sending them.

use super::*;

/// The mouse part of `event`, which must be a mouse event.
#[allow(unsafe_code)]
fn mouse(event: &INPUT) -> MOUSEINPUT {
    assert_eq!(event.r#type, INPUT_MOUSE);
    // SAFETY: a mouse event's union holds `mi`.
    unsafe { event.Anonymous.mi }
}

/// The keyboard part of `event`, which must be a keyboard event.
#[allow(unsafe_code)]
fn keyboard(event: &INPUT) -> KEYBDINPUT {
    assert_eq!(event.r#type, INPUT_KEYBOARD);
    // SAFETY: a keyboard event's union holds `ki`.
    unsafe { event.Anonymous.ki }
}

fn named(name: &str) -> Key {
    Key::from_name(name).unwrap()
}

fn key_event_of(name: &str, state: PressState) -> KEYBDINPUT {
    keyboard(&key_event(keys::code(named(name)).unwrap(), state))
}

#[test]
fn the_marker_is_non_zero_and_fits_32_bits() {
    assert_ne!(MARKER, 0);
    assert_eq!(marker(), 0x4143_4C4B);
}

fn moved(dx: i32, dy: i32) -> MOUSEINPUT {
    MOUSEINPUT {
        dx,
        dy,
        dwFlags: MOUSEEVENTF_MOVE,
        ..MOUSEINPUT::default()
    }
}

const STATES: [PressState; 2] = [PressState::Down, PressState::Up];

#[test]
fn every_event_carries_the_marker() {
    let buttons = Button::ALL
        .into_iter()
        .flat_map(|button| STATES.map(|state| button_event(button, state)));
    let others = [
        mouse_event(moved(1, 2)),
        absolute_event(AbsolutePosition { x: 1, y: 2 }),
        scroll_event(1),
    ];
    let mice: Vec<INPUT> = buttons.chain(others).collect();
    assert!(
        mice.iter()
            .all(|event| mouse(event).dwExtraInfo == marker())
    );
    let keys = ["a", "up", "pause", "numlock"]
        .into_iter()
        .flat_map(|name| STATES.map(|state| key_event_of(name, state)));
    assert!(keys.into_iter().all(|key| key.dwExtraInfo == marker()));
}

#[test]
fn buttons_press_and_release_with_their_own_flags() {
    use PressState::{Down, Up};
    let cases = [
        (Button::Left, Down, MOUSEEVENTF_LEFTDOWN, 0),
        (Button::Left, Up, MOUSEEVENTF_LEFTUP, 0),
        (Button::Right, Down, MOUSEEVENTF_RIGHTDOWN, 0),
        (Button::Right, Up, MOUSEEVENTF_RIGHTUP, 0),
        (Button::Middle, Down, MOUSEEVENTF_MIDDLEDOWN, 0),
        (Button::Middle, Up, MOUSEEVENTF_MIDDLEUP, 0),
        // Back and forward: Windows' X buttons 1 and 2.
        (Button::Side, Down, MOUSEEVENTF_XDOWN, 1),
        (Button::Side, Up, MOUSEEVENTF_XUP, 1),
        (Button::Extra, Down, MOUSEEVENTF_XDOWN, 2),
        (Button::Extra, Up, MOUSEEVENTF_XUP, 2),
    ];
    for (button, state, flags, data) in cases {
        let event = mouse(&button_event(button, state));
        let actual = (event.dwFlags, event.mouseData, event.dx, event.dy);
        assert_eq!(actual, (flags, data, 0, 0), "{button:?} {state:?}");
    }
}

#[test]
fn keys_go_by_scan_code_with_the_extended_flag_on_e0_keys() {
    let a = key_event_of("a", PressState::Down);
    assert_eq!((a.wVk, a.wScan, a.dwFlags), (0, 0x1e, KEYEVENTF_SCANCODE));
    let up = key_event_of("up", PressState::Up);
    let flags = KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP;
    assert_eq!((up.wVk, up.wScan, up.dwFlags), (0, 0x48, flags));
    let numlock = key_event_of("numlock", PressState::Down);
    assert_eq!((numlock.wScan, numlock.dwFlags), (0x45, KEYEVENTF_SCANCODE));
}

#[test]
fn pause_goes_by_its_virtual_key_alone() {
    let down = key_event_of("pause", PressState::Down);
    let up = key_event_of("pause", PressState::Up);
    assert_eq!((down.wVk, down.wScan, down.dwFlags), (0x13, 0, 0));
    assert_eq!((up.wVk, up.wScan, up.dwFlags), (0x13, 0, KEYEVENTF_KEYUP));
}

#[test]
fn a_key_windows_lacks_fails_to_press_and_its_release_sends_nothing() {
    let mut output = Output { _private: () };
    // KEY_MACRO: recordable on Linux, no key on Windows.
    let macro_key = Key::new(112).unwrap();
    let pressed = output.key(macro_key, PressState::Down);
    assert!(
        matches!(&pressed, Err(InputError::Write(err)) if err.to_string().contains("112")),
        "{pressed:?}"
    );
    assert!(output.key(macro_key, PressState::Up).is_ok());
}

#[test]
fn the_wheel_turns_120_a_notch_up_positive_and_saturates() {
    let turn = |notches| mouse(&scroll_event(notches));
    assert_eq!(turn(1).dwFlags, MOUSEEVENTF_WHEEL);
    assert_eq!(turn(1).mouseData.cast_signed(), 120);
    assert_eq!(turn(-3).mouseData.cast_signed(), -360);
    assert_eq!(turn(i32::MAX).mouseData.cast_signed(), i32::MAX);
    assert_eq!(turn(i32::MIN).mouseData.cast_signed(), i32::MIN);
}

#[test]
fn relative_moves_go_as_they_are() {
    let event = mouse(&mouse_event(moved(-7, 12)));
    assert_eq!(
        (event.dwFlags, event.dx, event.dy),
        (MOUSEEVENTF_MOVE, -7, 12)
    );
}

#[test]
fn absolute_events_span_the_virtual_desktop_with_the_value_given() {
    let flags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    for (x, y) in [(0, 0), (32_768, 100), (65_535, 65_535)] {
        let event = mouse(&absolute_event(AbsolutePosition { x, y }));
        let actual = (event.dwFlags, event.dx, event.dy);
        assert_eq!(actual, (flags, i32::from(x), i32::from(y)));
    }
}

/// Where Windows puts a virtual-desktop value on one axis, as the VM's
/// sweep measured it (`device_tests.rs`): the box's start plus
/// `floor(value * size / 65536)`.
fn lands(value: u16, start: i32, size: u32) -> i32 {
    let offset = u64::from(value) * u64::from(size) / 65_536;
    start + i32::try_from(offset).unwrap()
}

/// Core's position for a pixel `offset` into an axis of `size` pixels:
/// `round(offset * 65536 / size)`, as `Layout::absolute` gives it.
fn position(offset: u32, size: u32) -> u16 {
    let scaled = (u64::from(offset) * 65_536 + u64::from(size) / 2) / u64::from(size);
    u16::try_from(scaled.min(65_535)).unwrap()
}

#[test]
fn every_pixel_core_names_is_the_pixel_windows_lands_on() {
    // Up to 16,384 px, a 65,536 range still has a value for every pixel.
    for size in [
        1, 2, 3, 768, 1080, 1440, 1640, 1920, 2560, 4480, 7680, 16_384,
    ] {
        for pixel in 0..size {
            let sent = snap(position(pixel, size), size);
            assert_eq!(
                lands(sent, 0, size),
                pixel.cast_signed(),
                "pixel {pixel} of {size}"
            );
        }
    }
}

/// Core's position for the middle of pixel `offset`, as
/// `Layout::absolute_at` gives it for a floored recording's point.
fn middle(offset: u32, size: u32) -> u16 {
    let scaled = ((f64::from(offset) + 0.5) * 65_536.0 / f64::from(size)).round();
    u16::try_from(scaled as u64).unwrap()
}

#[test]
fn every_pixels_middle_lands_on_that_pixel() {
    // Two 1920 px screens side by side: the middle of 1919 stays on the
    // left one, and of 1920 on the right one.
    for size in [768, 1080, 1920, 3840, 7680, 16_384] {
        for pixel in 0..size {
            let sent = snap(middle(pixel, size), size);
            assert_eq!(
                lands(sent, 0, size),
                pixel.cast_signed(),
                "middle of {pixel} of {size}"
            );
        }
    }
}

#[test]
fn a_desktop_with_a_negative_origin_lands_on_the_pixel_meant() {
    // A 1920x1080 monitor left of and above a primary 2560x1440 one: the
    // virtual desktop's box starts at (-1920, -200) and is 4480x1640.
    let (left, top, width, height) = (-1920, -200, 4480_u32, 1640_u32);
    let points = [(-1920, -200), (-1, 879), (0, 0), (2559, 1439), (1280, 720)];
    for (x, y) in points {
        // The layout counts from the box's corner, as WN09 builds it.
        let meant = AbsolutePosition {
            x: position(u32::try_from(x - left).unwrap(), width),
            y: position(u32::try_from(y - top).unwrap(), height),
        };
        let sent = mouse(&absolute_event(on_pixels(meant, Some((width, height)))));
        let landed_x = lands(u16::try_from(sent.dx).unwrap(), left, width);
        let landed_y = lands(u16::try_from(sent.dy).unwrap(), top, height);
        assert_eq!((landed_x, landed_y), (x, y));
    }
}

#[test]
fn without_a_desktop_size_a_position_goes_as_it_is() {
    let position = AbsolutePosition { x: 34, y: 65_535 };
    assert_eq!(on_pixels(position, None), position);
}
