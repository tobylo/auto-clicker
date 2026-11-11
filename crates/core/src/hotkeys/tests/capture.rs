//! Capturing what the user presses: the first chord or key, once, and no
//! binding fires meanwhile.

use super::*;

const ESC: u16 = 1;
const MUTE: u16 = 113;
const BTN_LEFT: u16 = 272;

/// Feeds `steps` to a matcher capturing for `capture`.
fn capture(capture: Capture, steps: &[Step]) -> Vec<HotkeyEvent> {
    feed(&mut Matcher::capturing(capture), steps)
}

fn got(captured: Captured) -> Vec<HotkeyEvent> {
    vec![HotkeyEvent::Captured(captured)]
}

#[test]
fn a_hotkey_is_the_modifiers_held_and_the_key_pressed_and_comes_once() {
    let steps = [
        down(A, LEFTCTRL),
        down(B, LEFTSHIFT),
        down(A, F9),
        up(A, F9),
        down(A, F7),
    ];
    let chord = Captured::Hotkey(binding("Ctrl+Shift+F9"));
    assert_eq!(capture(Capture::Hotkey, &steps), got(chord));
}

#[test]
fn modifiers_alone_capture_no_hotkey() {
    let steps = [down(A, LEFTCTRL), up(A, LEFTCTRL), down(A, LEFTSHIFT)];
    assert_eq!(capture(Capture::Hotkey, &steps), []);
    let after = [down(A, LEFTSHIFT), up(A, LEFTSHIFT), down(A, F9)];
    assert_eq!(
        capture(Capture::Hotkey, &after),
        got(Captured::Hotkey(binding("F9")))
    );
}

#[test]
fn a_bound_hotkey_is_captured_and_does_not_fire() {
    let mut matcher = Matcher::capturing(Capture::Hotkey);
    let sent = feed(&mut matcher, &[down(A, LEFTCTRL), down(A, F6)]);
    assert_eq!(sent, got(Captured::Hotkey(binding("Ctrl+F6"))));
}

#[test]
fn esc_cancels_either_capture() {
    for purpose in [Capture::Hotkey, Capture::Key] {
        let sent = capture(purpose, &[down(A, LEFTCTRL), down(A, ESC)]);
        let expected = match purpose {
            // A key capture takes the first key, Ctrl.
            Capture::Key => got(Captured::Key(Key::new(LEFTCTRL).unwrap())),
            Capture::Hotkey => got(Captured::Cancelled),
        };
        assert_eq!(sent, expected, "{purpose:?}");
    }
    assert_eq!(
        capture(Capture::Key, &[down(A, ESC)]),
        got(Captured::Cancelled)
    );
}

#[test]
fn a_key_without_a_name_is_no_hotkey_but_is_a_clicker_key() {
    assert_eq!(
        capture(Capture::Hotkey, &[down(A, MUTE)]),
        got(Captured::Unnamed)
    );
    let mute = Key::new(MUTE).unwrap();
    assert_eq!(
        capture(Capture::Key, &[down(A, MUTE)]),
        got(Captured::Key(mute))
    );
}

#[test]
fn the_clicker_key_is_the_first_key_down_a_modifier_too() {
    let shift = Key::new(LEFTSHIFT).unwrap();
    assert_eq!(
        capture(Capture::Key, &[down(A, LEFTSHIFT), down(A, F9)]),
        got(Captured::Key(shift))
    );
}

#[test]
fn buttons_releases_and_repeats_capture_nothing() {
    let steps = [down(A, BTN_LEFT), repeat(A, F9), up(A, F9)];
    assert_eq!(capture(Capture::Key, &steps), []);
    assert_eq!(capture(Capture::Hotkey, &steps), []);
}

#[test]
fn a_press_older_than_a_read_is_not_captured() {
    // A was read holding nothing at 500; B's F9 from 450 may have come with
    // a modifier A held then.
    let steps = [Step::Read(A, &[], 500), at(450, down(B, F9)), down(B, F7)];
    assert_eq!(
        capture(Capture::Hotkey, &steps),
        got(Captured::Hotkey(binding("F7")))
    );
}

#[test]
fn a_binding_is_made_of_a_named_key_that_is_no_modifier() {
    let ctrl = Modifiers::NONE.with(Modifier::Ctrl);
    assert_eq!(Binding::new(ctrl, key("f9")), Some(binding("Ctrl+F9")));
    assert_eq!(Binding::new(ctrl, Key::new(MUTE).unwrap()), None);
    assert_eq!(Binding::new(Modifiers::NONE, key("leftshift")), None);
}
