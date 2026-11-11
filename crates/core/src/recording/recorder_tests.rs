//! The recorder against hand-made input: what it keeps, what it leaves out
//! and what it adds, as the saved recording shows it.

use std::time::Duration;

use super::*;
use crate::hotkeys::{Action, Binding, Bindings, DeviceId};
use crate::input::{Button, Held};
use crate::recording::tests::{created, key};

const KEYBOARD: DeviceId = DeviceId(1);
const MOUSE: DeviceId = DeviceId(2);
const OTHER_KEYBOARD: DeviceId = DeviceId(3);

const CTRL: u16 = 29;
const W: u16 = 17;
const F6: u16 = 64;
const F7: u16 = 65;
const LEFT: Held = Held::Button(Button::Left);
const RIGHT: Held = Held::Button(Button::Right);

/// The recording starts 100 s into the event clock.
const START: Duration = Duration::from_secs(100);

/// `us` microseconds after the start, on the event clock.
fn t(us: u64) -> Duration {
    START + Duration::from_micros(us)
}

/// Ctrl+F6, Ctrl+F7 and Ctrl+F8: one modifier, which keeps these cases short.
fn bindings() -> Bindings {
    let ctrl = |key: &str| Binding::parse(&format!("Ctrl+{key}")).unwrap();
    Bindings {
        toggle_clicking: ctrl("F6"),
        toggle_recording: ctrl("F7"),
        toggle_playback: ctrl("F8"),
    }
}

fn recorder() -> Recorder {
    Recorder::new(bindings(), START, created(), None)
}

fn press(held: Held) -> Change {
    Change::Press(held)
}

fn release(held: Held) -> Change {
    Change::Release(held)
}

fn moved(dx: i32, dy: i32) -> Change {
    Change::Move { dx, dy }
}

fn wheel(notches: i32) -> Change {
    Change::Wheel(notches)
}

/// A recording's events as (microseconds, change).
fn changes(recording: &Recording) -> Vec<(u64, Change)> {
    recording
        .events()
        .iter()
        .map(|event| (event.at, event.change))
        .collect()
}

fn finish_at(recorder: Recorder, end: Duration) -> Recording {
    recorder.finish(Ending::At(end)).unwrap().unwrap()
}

fn events_at(recorder: Recorder, end: Duration) -> Vec<(u64, Change)> {
    changes(&finish_at(recorder, end))
}

fn tap(recorder: &mut Recorder, device: DeviceId, code: u16, [down, up]: [u64; 2]) {
    recorder.key(device, KeyInput::press(code), t(down));
    recorder.key(device, KeyInput::release(code), t(up));
}

fn button_code(held: Held) -> u16 {
    crate::recording::codes::code(held)
}

#[test]
fn motion_and_wheel_are_summed_per_millisecond_exactly() {
    let mut recorder = recorder();
    // An 8 kHz mouse: eight reports a millisecond.
    for n in 0..16 {
        recorder.motion(t(n * 125), 1, -2);
        if n == 3 || n == 7 {
            recorder.wheel(t(n * 125 + 10), 1);
        }
    }
    recorder.motion(t(2_000), 5, 0);
    assert_eq!(
        events_at(recorder, t(3_000)),
        [
            (0, moved(8, -16)),
            (0, wheel(2)),
            (1_000, moved(8, -16)),
            (2_000, moved(5, 0))
        ]
    );
}

#[test]
fn a_click_keeps_its_place_among_the_moves() {
    let mut recorder = recorder();
    recorder.motion(t(100), 3, 0);
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(400));
    recorder.motion(t(600), 0, 4);
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(700));
    assert_eq!(
        events_at(recorder, t(1_000)),
        [
            (100, moved(3, 0)),
            (400, press(LEFT)),
            (600, moved(0, 4)),
            (700, release(LEFT))
        ]
    );
}

#[test]
fn motion_that_sums_to_nothing_is_left_out() {
    let mut recorder = recorder();
    recorder.motion(t(0), 3, 0);
    recorder.motion(t(500), -3, 0);
    recorder.wheel(t(600), 2);
    recorder.wheel(t(700), -2);
    tap(&mut recorder, KEYBOARD, W, [2_000, 3_000]);
    assert_eq!(
        events_at(recorder, t(4_000)),
        [(2_000, press(key(W))), (3_000, release(key(W)))]
    );
}

#[test]
fn a_sum_that_would_overflow_starts_a_new_move() {
    let mut recorder = recorder();
    recorder.motion(t(0), i32::MAX, 0);
    recorder.motion(t(10), 1, 0);
    assert_eq!(
        events_at(recorder, t(100)),
        [(0, moved(i32::MAX, 0)), (10, moved(1, 0))]
    );
}

#[test]
fn a_key_held_on_two_devices_is_pressed_once_and_released_by_the_last() {
    let mut recorder = recorder();
    recorder.key(KEYBOARD, KeyInput::press(W), t(10));
    recorder.key(OTHER_KEYBOARD, KeyInput::press(W), t(20));
    recorder.key(KEYBOARD, KeyInput::press(W), t(25));
    recorder.key(KEYBOARD, KeyInput::release(W), t(30));
    recorder.key(KEYBOARD, KeyInput::release(W), t(35));
    recorder.key(OTHER_KEYBOARD, KeyInput::release(W), t(40));
    assert_eq!(
        events_at(recorder, t(100)),
        [(10, press(key(W))), (40, release(key(W)))]
    );
}

#[test]
fn a_release_without_its_press_and_codes_playback_cannot_send_are_dropped() {
    let mut recorder = recorder();
    recorder.key(KEYBOARD, KeyInput::release(W), t(10));
    for code in [84, 0x115, 0x14a, 300] {
        tap(&mut recorder, KEYBOARD, code, [20, 30]);
    }
    assert_eq!(recorder.finish(Ending::At(t(100))), Ok(None));
}

#[test]
fn everything_still_down_is_released_at_the_stop() {
    let mut recorder = recorder();
    recorder.key(KEYBOARD, KeyInput::press(W), t(10));
    recorder.key(MOUSE, KeyInput::press(button_code(RIGHT)), t(20));
    let recording = finish_at(recorder, t(5_000));
    assert_eq!(recording.duration_us(), 5_000);
    assert_eq!(
        changes(&recording),
        [
            (10, press(key(W))),
            (20, press(RIGHT)),
            (5_000, release(RIGHT)),
            (5_000, release(key(W)))
        ]
    );
}

#[test]
fn an_unplugged_device_releases_what_it_held_then() {
    let mut recorder = recorder();
    recorder.key(KEYBOARD, KeyInput::press(W), t(10));
    recorder.key(OTHER_KEYBOARD, KeyInput::press(CTRL), t(20));
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(30));
    recorder.device_lost(MOUSE, t(500));
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(600));
    assert_eq!(
        events_at(recorder, t(1_000)),
        [
            (10, press(key(W))),
            (20, press(key(CTRL))),
            (30, press(LEFT)),
            (500, release(LEFT)),
            (1_000, release(key(W))),
            (1_000, release(key(CTRL))),
        ]
    );
}

#[test]
fn a_resync_releases_what_came_up_and_presses_what_went_down_in_the_gap() {
    let mut recorder = recorder();
    recorder.key(KEYBOARD, KeyInput::press(W), t(10));
    recorder.key(KEYBOARD, KeyInput::press(CTRL), t(20));
    // Events were lost: W came up, F6 went down.
    recorder.resync(KEYBOARD, |code| [CTRL, F6].contains(&code), t(300));
    assert_eq!(
        events_at(recorder, t(1_000)),
        [
            (10, press(key(W))),
            (20, press(key(CTRL))),
            (300, release(key(W))),
            (300, press(key(F6))),
            (1_000, release(key(CTRL))),
            (1_000, release(key(F6))),
        ]
    );
}

#[test]
fn keys_held_when_a_tray_start_begins_are_pressed_at_the_start() {
    let mut recorder = recorder();
    recorder.resync(KEYBOARD, |code| code == W, t(0));
    recorder.resync(MOUSE, |code| code == button_code(LEFT), t(0));
    recorder.key(KEYBOARD, KeyInput::release(W), t(2_000));
    assert_eq!(
        events_at(recorder, t(3_000)),
        [
            (0, press(key(W))),
            (0, press(LEFT)),
            (2_000, release(key(W))),
            (3_000, release(LEFT))
        ]
    );
}

#[test]
fn the_start_hotkeys_own_keys_are_left_out_until_they_come_up() {
    let started_by = bindings().toggle_recording;
    let mut recorder = Recorder::new(bindings(), START, created(), Some(started_by));
    // Ctrl+F7 is still down at the start, and W too.
    recorder.resync(KEYBOARD, |code| [CTRL, F7, W].contains(&code), t(0));
    recorder.key(KEYBOARD, KeyInput::release(F7), t(50));
    recorder.key(KEYBOARD, KeyInput::release(CTRL), t(60));
    // Pressed again later, Ctrl is the user's own.
    tap(&mut recorder, KEYBOARD, CTRL, [1_000, 1_100]);
    assert_eq!(
        events_at(recorder, t(2_000)),
        [
            (0, press(key(W))),
            (1_000, press(key(CTRL))),
            (1_100, release(key(CTRL))),
            (2_000, release(key(W)))
        ]
    );
}

#[test]
fn a_start_hotkey_key_seen_up_by_a_resync_is_no_longer_left_out() {
    let started_by = bindings().toggle_recording;
    let mut recorder = Recorder::new(bindings(), START, created(), Some(started_by));
    recorder.resync(KEYBOARD, |code| code == CTRL, t(0));
    // Events were lost while Ctrl came up and went down again.
    recorder.resync(KEYBOARD, |_| false, t(100));
    recorder.resync(KEYBOARD, |code| code == CTRL, t(200));
    assert_eq!(
        events_at(recorder, t(300)),
        [(200, press(key(CTRL))), (300, release(key(CTRL)))]
    );
}

#[test]
fn a_device_read_first_that_holds_none_of_the_start_hotkey_keeps_it_left_out() {
    let started_by = bindings().toggle_recording;
    let mut recorder = Recorder::new(bindings(), START, created(), Some(started_by));
    // A mouse holding nothing is read before the keyboard holding Ctrl+F7.
    recorder.resync(MOUSE, |_| false, t(0));
    recorder.resync(KEYBOARD, |code| [CTRL, F7].contains(&code), t(0));
    recorder.key(KEYBOARD, KeyInput::release(F7), t(50));
    recorder.key(KEYBOARD, KeyInput::release(CTRL), t(60));
    tap(&mut recorder, KEYBOARD, W, [100, 200]);
    assert_eq!(
        events_at(recorder, t(300)),
        [(100, press(key(W))), (200, release(key(W)))]
    );
}

#[test]
fn the_start_hotkeys_other_side_and_other_devices_are_recorded() {
    let started_by = bindings().toggle_recording;
    let mut recorder = Recorder::new(bindings(), START, created(), Some(started_by));
    recorder.resync(KEYBOARD, |code| [CTRL, F7].contains(&code), t(0));
    recorder.resync(OTHER_KEYBOARD, |_| false, t(0));
    // Right Ctrl was never down; Ctrl on another keyboard is its own.
    tap(&mut recorder, KEYBOARD, 97, [100, 200]);
    tap(&mut recorder, OTHER_KEYBOARD, CTRL, [300, 400]);
    recorder.key(KEYBOARD, KeyInput::release(CTRL), t(500));
    assert_eq!(
        events_at(recorder, t(600)),
        [
            (100, press(key(97))),
            (200, release(key(97))),
            (300, press(key(CTRL))),
            (400, release(key(CTRL)))
        ]
    );
}

#[test]
fn a_hotkey_still_held_after_lost_events_stays_out() {
    let mut recorder = recorder();
    recorder.resync(KEYBOARD, |_| false, t(0));
    recorder.key(KEYBOARD, KeyInput::press(CTRL), t(100));
    recorder.key(
        KEYBOARD,
        KeyInput::press(F6).firing(Action::ToggleClicking),
        t(200),
    );
    // Events were lost with Ctrl+F6 still down, then they come up.
    recorder.resync(KEYBOARD, |code| [CTRL, F6].contains(&code), t(300));
    recorder.resync(KEYBOARD, |_| false, t(400));
    tap(&mut recorder, KEYBOARD, W, [500, 600]);
    assert_eq!(
        events_at(recorder, t(700)),
        [(500, press(key(W))), (600, release(key(W)))]
    );
}

#[test]
fn an_unplugged_device_takes_its_exclusions_with_it() {
    let started_by = bindings().toggle_recording;
    let mut recorder = Recorder::new(bindings(), START, created(), Some(started_by));
    recorder.resync(KEYBOARD, |code| code == CTRL, t(0));
    recorder.device_lost(KEYBOARD, t(100));
    // Plugged in again under the same id: its Ctrl is the user's now.
    recorder.resync(KEYBOARD, |_| false, t(200));
    tap(&mut recorder, KEYBOARD, CTRL, [300, 400]);
    assert_eq!(
        events_at(recorder, t(500)),
        [(300, press(key(CTRL))), (400, release(key(CTRL)))]
    );
}

#[test]
fn a_device_added_after_the_start_records_its_held_ctrl() {
    let started_by = bindings().toggle_recording;
    let mut recorder = Recorder::new(bindings(), START, created(), Some(started_by));
    recorder.resync(KEYBOARD, |code| [CTRL, F7].contains(&code), t(0));
    recorder.start_read();
    // Plugged in later with Ctrl held: the user's own Ctrl.
    recorder.resync(OTHER_KEYBOARD, |code| code == CTRL, t(1_000));
    assert_eq!(
        events_at(recorder, t(2_000)),
        [(1_000, press(key(CTRL))), (2_000, release(key(CTRL)))]
    );
}

#[test]
fn the_stop_hotkey_is_left_out_and_ends_the_recording_at_its_press() {
    let mut recorder = recorder();
    tap(&mut recorder, KEYBOARD, W, [0, 1_000]);
    recorder.key(KEYBOARD, KeyInput::press(CTRL), t(5_000));
    recorder.motion(t(5_100), 1, 1);
    recorder.key(
        KEYBOARD,
        KeyInput::press(F7).firing(Action::ToggleRecording),
        t(5_200),
    );
    // What comes after the stop is not recorded.
    recorder.key(KEYBOARD, KeyInput::release(F7), t(5_300));
    recorder.key(KEYBOARD, KeyInput::release(CTRL), t(5_400));
    tap(&mut recorder, KEYBOARD, W, [5_500, 5_600]);
    recorder.device_lost(KEYBOARD, t(5_700));
    let recording = finish_at(recorder, t(9_000));
    assert_eq!(recording.duration_us(), 5_200);
    assert_eq!(
        changes(&recording),
        [
            (0, press(key(W))),
            (1_000, release(key(W))),
            (5_100, moved(1, 1))
        ]
    );
}

#[test]
fn ctrl_held_to_sprint_before_the_stop_hotkey_is_kept_and_released() {
    let mut recorder = recorder();
    recorder.key(KEYBOARD, KeyInput::press(CTRL), t(0));
    recorder.key(KEYBOARD, KeyInput::press(W), t(100_000));
    recorder.key(
        KEYBOARD,
        KeyInput::press(F7).firing(Action::ToggleRecording),
        t(3_000_000),
    );
    assert_eq!(
        events_at(recorder, t(4_000_000)),
        [
            (0, press(key(CTRL))),
            (100_000, press(key(W))),
            (3_000_000, release(key(W))),
            (3_000_000, release(key(CTRL))),
        ]
    );
}

#[test]
fn a_modifier_pressed_long_before_the_hotkey_is_kept_even_with_only_motion_since() {
    let mut recorder = recorder();
    recorder.key(KEYBOARD, KeyInput::press(CTRL), t(0));
    recorder.motion(t(500_000), 4, 0);
    recorder.key(
        KEYBOARD,
        KeyInput::press(F7).firing(Action::ToggleRecording),
        t(1_000_001),
    );
    let kept = events_at(recorder, t(2_000_000));
    assert_eq!(kept.first(), Some(&(0, press(key(CTRL)))));

    let mut recorder = self::recorder();
    recorder.key(KEYBOARD, KeyInput::press(CTRL), t(0));
    recorder.motion(t(500_000), 4, 0);
    recorder.key(
        KEYBOARD,
        KeyInput::press(F7).firing(Action::ToggleRecording),
        t(1_000_000),
    );
    assert_eq!(events_at(recorder, t(2_000_000)), [(500_000, moved(4, 0))]);
}

#[test]
fn a_modifier_with_a_release_after_it_is_kept() {
    let mut recorder = recorder();
    recorder.key(KEYBOARD, KeyInput::press(W), t(0));
    recorder.key(KEYBOARD, KeyInput::press(CTRL), t(100));
    recorder.key(KEYBOARD, KeyInput::release(W), t(200));
    recorder.key(
        KEYBOARD,
        KeyInput::press(F7).firing(Action::ToggleRecording),
        t(300),
    );
    assert_eq!(
        events_at(recorder, t(1_000)),
        [
            (0, press(key(W))),
            (100, press(key(CTRL))),
            (200, release(key(W))),
            (300, release(key(CTRL)))
        ]
    );
}

#[test]
fn a_stop_stamped_before_the_last_event_ends_after_it() {
    let mut recorder = recorder();
    recorder.key(KEYBOARD, KeyInput::press(W), t(500));
    let recording = finish_at(recorder, t(400));
    assert_eq!(recording.duration_us(), 500);
    assert_eq!(
        changes(&recording),
        [(500, press(key(W))), (500, release(key(W)))]
    );
}

#[test]
fn other_hotkeys_during_a_recording_are_left_out_and_it_goes_on() {
    let mut recorder = recorder();
    tap(&mut recorder, KEYBOARD, W, [0, 100]);
    // Ctrl+F6 with both Ctrls, pressed for it.
    recorder.key(KEYBOARD, KeyInput::press(CTRL), t(1_000));
    recorder.key(OTHER_KEYBOARD, KeyInput::press(97), t(1_100));
    recorder.key(
        KEYBOARD,
        KeyInput::press(F6).firing(Action::ToggleClicking),
        t(1_200),
    );
    recorder.key(KEYBOARD, KeyInput::release(F6), t(1_300));
    recorder.key(OTHER_KEYBOARD, KeyInput::release(97), t(1_350));
    recorder.key(KEYBOARD, KeyInput::release(CTRL), t(1_400));
    tap(&mut recorder, KEYBOARD, W, [2_000, 2_100]);
    assert_eq!(
        events_at(recorder, t(3_000)),
        [
            (0, press(key(W))),
            (100, release(key(W))),
            (2_000, press(key(W))),
            (2_100, release(key(W)))
        ]
    );
}

#[test]
fn a_hotkey_with_a_custom_binding_is_matched_by_its_action() {
    let bindings = Bindings {
        toggle_recording: Binding::parse("Shift+F9").unwrap(),
        ..bindings()
    };
    let mut recorder = Recorder::new(bindings, START, created(), None);
    recorder.key(KEYBOARD, KeyInput::press(CTRL), t(0));
    recorder.key(KEYBOARD, KeyInput::press(42), t(10));
    recorder.key(
        KEYBOARD,
        KeyInput::press(67).firing(Action::ToggleRecording),
        t(20),
    );
    // Ctrl is no part of Shift+F9: kept, and released at the stop.
    assert_eq!(
        events_at(recorder, t(100)),
        [(0, press(key(CTRL))), (20, release(key(CTRL)))]
    );
}

#[test]
fn times_never_go_back_and_nothing_counts_past_the_time_limit() {
    let mut recorder = recorder();
    recorder.key(KEYBOARD, KeyInput::press(W), t(500));
    // Another device's event read later but stamped earlier.
    recorder.key(OTHER_KEYBOARD, KeyInput::press(CTRL), t(400));
    recorder.motion(t(300), 1, 0);
    let limit = MAX_DURATION_US;
    tap(&mut recorder, KEYBOARD, F6, [limit + 1, limit + 2]);
    let recording = recorder.finish(Ending::At(t(limit + 50))).unwrap().unwrap();
    assert_eq!(recording.duration_us(), limit);
    assert_eq!(
        changes(&recording),
        [
            (500, press(key(W))),
            (500, press(key(CTRL))),
            (500, moved(1, 0)),
            (limit, release(key(W))),
            (limit, release(key(CTRL))),
        ]
    );
}

#[test]
fn input_from_before_the_start_counts_from_the_start() {
    let mut recorder = recorder();
    recorder.key(
        KEYBOARD,
        KeyInput::press(W),
        START - Duration::from_millis(1),
    );
    assert_eq!(
        events_at(recorder, t(10)),
        [(0, press(key(W))), (10, release(key(W)))]
    );
}

#[test]
fn a_recording_with_nothing_in_it_is_not_saved() {
    assert_eq!(recorder().finish(Ending::At(t(5_000))), Ok(None));
    let mut stopped_at_once = recorder();
    stopped_at_once.key(
        KEYBOARD,
        KeyInput::press(F7).firing(Action::ToggleRecording),
        t(0),
    );
    assert_eq!(stopped_at_once.finish(Ending::At(t(10))), Ok(None));
}

#[test]
fn random_input_always_makes_a_valid_recording() {
    let mut rng = fastrand::Rng::with_seed(7);
    let devices = [KEYBOARD, MOUSE, OTHER_KEYBOARD];
    let codes = [CTRL, W, F6, F7, 97, 0x110, 0x111, 84];
    for round in 0..200 {
        let mut recorder = recorder();
        let mut now = 0_u64;
        for _ in 0..rng.usize(0..60) {
            now += rng.u64(0..3_000);
            // Up to 2 ms out of order, as across devices.
            let at = t(now.saturating_sub(rng.u64(0..2_000)));
            let device = devices[rng.usize(..devices.len())];
            match rng.u8(0..10) {
                0..=4 => {
                    let code = codes[rng.usize(..codes.len())];
                    let fired = (rng.u8(0..20) == 0).then_some(Action::ToggleClicking);
                    recorder.key(
                        device,
                        KeyInput {
                            fired,
                            ..KeyInput::new(code, rng.bool())
                        },
                        at,
                    );
                }
                5 | 6 => recorder.motion(at, rng.i32(-5..5), rng.i32(-5..5)),
                7 => recorder.wheel(at, rng.i32(-2..2)),
                8 => recorder.device_lost(device, at),
                _ => {
                    let held = rng.u16(..);
                    recorder.resync(device, |code| code % 7 == held % 7, at);
                }
            }
        }
        let ending = if rng.bool() {
            Ending::At(t(now))
        } else {
            Ending::Tray {
                at: t(now),
                menu_opened: Some(t(now.saturating_sub(rng.u64(0..20_000)))),
            }
        };
        let result = recorder.finish(ending);
        assert!(result.is_ok(), "round {round}: {result:?}");
    }
}

// ---------------------------------------------------------------------------
// The trip to the tray

/// A recording of mining in a game: walk, then click and look around, with
/// no key at the end, followed by the trip to the tray: motion, a click on
/// the icon, motion, a click on "Stop recording". The look around just
/// before the trip is motion leading to it, and goes with it.
fn mining_then_tray_trip(recorder: &mut Recorder) {
    tap(recorder, KEYBOARD, W, [0, 1_000_000]);
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(1_500_000));
    recorder.motion(t(1_600_000), 20, 0);
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(2_500_000));
    recorder.motion(t(3_000_000), -5, 5);
    // The trip.
    recorder.motion(t(4_000_000), 300, -200);
    recorder.motion(t(4_100_000), 50, -20);
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(4_200_000));
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(4_250_000));
    recorder.motion(t(4_600_000), 0, -40);
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(5_000_000));
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(5_050_000));
}

fn mining() -> Vec<(u64, Change)> {
    vec![
        (0, press(key(W))),
        (1_000_000, release(key(W))),
        (1_500_000, press(LEFT)),
        (1_600_000, moved(20, 0)),
        (2_500_000, release(LEFT)),
    ]
}

fn tray_stop(recorder: Recorder, menu_opened: Option<u64>) -> Recording {
    let ending = Ending::Tray {
        at: t(5_100_000),
        menu_opened: menu_opened.map(t),
    };
    recorder.finish(ending).unwrap().unwrap()
}

#[test]
fn the_tray_trip_is_cut_from_the_click_that_opened_the_menu() {
    let mut recorder = recorder();
    mining_then_tray_trip(&mut recorder);
    let recording = tray_stop(recorder, Some(4_300_000));
    assert_eq!(changes(&recording), mining());
    assert_eq!(recording.duration_us(), 3_000_000);
}

#[test]
fn without_word_from_the_menu_the_last_two_clicks_are_the_trip() {
    let mut recorder = recorder();
    mining_then_tray_trip(&mut recorder);
    let recording = tray_stop(recorder, None);
    assert_eq!(changes(&recording), mining());
}

#[test]
fn a_press_dragged_to_the_menu_item_is_one_click() {
    let mut recorder = recorder();
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(0));
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(100_000));
    recorder.motion(t(21_000_000), 300, -200);
    // 20 s later: pressed on the icon (the menu opens), dragged to the
    // item, let go.
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(21_200_000));
    recorder.motion(t(21_400_000), 0, -40);
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(21_600_000));
    let ending = Ending::Tray {
        at: t(21_700_000),
        menu_opened: Some(t(21_250_000)),
    };
    let recording = recorder.finish(ending).unwrap().unwrap();
    assert_eq!(
        changes(&recording),
        [(0, press(LEFT)), (100_000, release(LEFT))]
    );
}

#[test]
fn a_menu_held_open_long_still_marks_the_trip() {
    let mut recorder = recorder();
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(0));
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(100_000));
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(1_000_000));
    // The menu stays open 30 s before the drag ends on Stop recording.
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(31_000_000));
    let ending = Ending::Tray {
        at: t(31_100_000),
        menu_opened: Some(t(1_050_000)),
    };
    let recording = recorder.finish(ending).unwrap().unwrap();
    assert_eq!(
        changes(&recording),
        [(0, press(LEFT)), (100_000, release(LEFT))]
    );
}

#[test]
fn the_trip_never_reaches_past_a_key() {
    let mut recorder = recorder();
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(0));
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(10));
    tap(&mut recorder, KEYBOARD, W, [1_000, 2_000]);
    recorder.motion(t(3_000), 5, 5);
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(4_000));
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(4_100));
    // The menu "opened" before W: that opening is not this trip's.
    let ending = Ending::Tray {
        at: t(5_000),
        menu_opened: Some(t(500)),
    };
    let recording = recorder.finish(ending).unwrap().unwrap();
    assert_eq!(
        changes(&recording),
        [
            (0, press(LEFT)),
            (10, release(LEFT)),
            (1_000, press(key(W))),
            (2_000, release(key(W)))
        ]
    );
    assert_eq!(recording.duration_us(), 3_000);
}

#[test]
fn a_button_held_through_the_trip_is_released_where_the_trip_began() {
    let mut recorder = recorder();
    recorder.key(MOUSE, KeyInput::press(button_code(RIGHT)), t(0));
    recorder.motion(t(1_000), 10, 0);
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(2_000));
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(2_100));
    recorder.key(MOUSE, KeyInput::release(button_code(RIGHT)), t(2_200));
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(3_000));
    let recording = tray_stop(recorder, None);
    assert_eq!(
        changes(&recording),
        [(0, press(RIGHT)), (1_000, release(RIGHT))]
    );
}

#[test]
fn a_tray_stop_with_no_click_cuts_nothing() {
    let mut recorder = recorder();
    tap(&mut recorder, KEYBOARD, W, [0, 1_000]);
    recorder.motion(t(2_000), 1, 1);
    let recording = tray_stop(recorder, Some(1_500));
    assert_eq!(
        changes(&recording),
        [
            (0, press(key(W))),
            (1_000, release(key(W))),
            (2_000, moved(1, 1))
        ]
    );
    assert_eq!(recording.duration_us(), 5_100_000);
}

#[path = "recorder_position_tests.rs"]
mod positions;
