//! The recorder's positions: which it keeps, where they go among the input
//! events, and that no report can make a recording fail.

use super::*;
use crate::input::Key;
use crate::recording::Track;
use crate::recording::tests::positions::screen;
use crate::recording::tests::positions::{at, two_screens};
use crate::recording::{Layout, MAX_OUTPUTS, Point, event_bytes};

fn positioned() -> Recorder {
    recorder().with_layout(two_screens())
}

/// A position report at `us`, made while the layout was the recording's.
fn report(recorder: &mut Recorder, us: u64, x: u32, y: u32) {
    let layout = two_screens();
    recorder.position(t(us), Point { x, y }, Some(&layout));
}

fn click(recorder: &mut Recorder, [down, up]: [u64; 2]) {
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(down));
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(up));
}

fn other_layout() -> Layout {
    Layout::new(vec![screen([0, 0, 2048, 1280], 1250)]).unwrap()
}

#[test]
fn positions_are_written_in_version_2_with_the_layout_at_the_start() {
    let mut recorder = positioned();
    recorder.motion(t(100), 3, 0);
    report(&mut recorder, 900, 10, 20);
    click(&mut recorder, [5_000, 6_000]);
    let recording = finish_at(recorder, t(7_000));
    assert_eq!(recording.layout(), Some(&two_screens()));
    assert_eq!(
        changes(&recording),
        [
            (100, moved(3, 0)),
            (900, at(10, 20)),
            (5_000, press(LEFT)),
            (6_000, release(LEFT)),
        ]
    );
}

#[test]
fn an_input_event_read_late_goes_before_the_positions_stamped_after_it() {
    let mut recorder = positioned();
    recorder.motion(t(100), 3, 0);
    report(&mut recorder, 1_500, 10, 20);
    // The click was made at 1.2 ms, but read after the position.
    click(&mut recorder, [1_200, 9_000]);
    assert_eq!(
        events_at(recorder, t(10_000)),
        [
            (100, moved(3, 0)),
            (1_200, press(LEFT)),
            (1_500, at(10, 20)),
            (9_000, release(LEFT)),
        ]
    );
}

#[test]
fn an_input_event_read_late_still_never_goes_before_an_input_event() {
    let mut recorder = positioned();
    recorder.key(KEYBOARD, KeyInput::press(W), t(2_000));
    report(&mut recorder, 2_500, 10, 20);
    // Stamped before the press already written: moved forward to it, and
    // still before the later position.
    click(&mut recorder, [1_000, 3_000]);
    recorder.key(KEYBOARD, KeyInput::release(W), t(4_000));
    assert_eq!(
        events_at(recorder, t(5_000)),
        [
            (0, at(10, 20)),
            (2_000, press(key(W))),
            (2_000, press(LEFT)),
            (3_000, release(LEFT)),
            (4_000, release(key(W))),
        ]
    );
}

#[test]
fn a_motion_buckets_move_goes_before_a_position_stamped_after_its_start() {
    let mut recorder = positioned();
    recorder.motion(t(100), 3, 0);
    report(&mut recorder, 400, 10, 20);
    // Same bucket as the first motion; written once the click ends it.
    recorder.motion(t(600), 2, 0);
    report(&mut recorder, 1_700, 12, 20);
    click(&mut recorder, [2_000, 3_000]);
    assert_eq!(
        events_at(recorder, t(4_000)),
        [
            (100, moved(5, 0)),
            (400, at(10, 20)),
            (1_700, at(12, 20)),
            (2_000, press(LEFT)),
            (3_000, release(LEFT)),
        ]
    );
}

#[test]
fn one_position_per_millisecond_is_kept_the_latest() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    for (us, x) in [(2_000, 1), (2_300, 2), (2_999, 3), (3_000, 4), (3_500, 5)] {
        report(&mut recorder, us, x, 0);
    }
    assert_eq!(
        events_at(recorder, t(4_000)),
        [(0, moved(1, 0)), (2_999, at(3, 0)), (3_500, at(5, 0))]
    );
}

#[test]
fn a_click_or_scroll_ends_the_millisecond_and_the_first_position_after_it_is_kept() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    report(&mut recorder, 2_000, 1, 0);
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(2_100));
    // The first after the press is kept as it is, even within its
    // millisecond; the next one opens a millisecond of its own.
    report(&mut recorder, 2_300, 2, 0);
    report(&mut recorder, 2_400, 3, 0);
    report(&mut recorder, 2_500, 4, 0);
    recorder.wheel(t(5_000), 1);
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(6_000));
    report(&mut recorder, 6_100, 5, 0);
    report(&mut recorder, 6_200, 6, 0);
    assert_eq!(
        events_at(recorder, t(7_000)),
        [
            (0, moved(1, 0)),
            (2_000, at(1, 0)),
            (2_100, press(LEFT)),
            (2_300, at(2, 0)),
            (2_500, at(4, 0)),
            (5_000, wheel(1)),
            (6_000, release(LEFT)),
            (6_100, at(5, 0)),
            (6_200, at(6, 0)),
        ]
    );
}

#[test]
fn a_click_read_late_before_an_open_position_makes_it_the_first_after_the_click() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    report(&mut recorder, 2_000, 1, 0);
    // Made at 1.9 ms, read after the position at 2 ms.
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(1_900));
    report(&mut recorder, 2_300, 2, 0);
    // A millisecond of its own: the next one replaces it.
    report(&mut recorder, 2_400, 3, 0);
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(9_000));
    assert_eq!(
        events_at(recorder, t(10_000)),
        [
            (0, moved(1, 0)),
            (1_900, press(LEFT)),
            (2_000, at(1, 0)),
            (2_400, at(3, 0)),
            (9_000, release(LEFT)),
        ]
    );
}

#[test]
fn a_click_read_late_inside_a_millisecond_keeps_the_position_before_it() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    report(&mut recorder, 2_000, 1, 0);
    report(&mut recorder, 2_200, 2, 0);
    report(&mut recorder, 2_600, 3, 0);
    // Made at 2.1 ms, read after all three: 2 ms is the last before it and
    // 2.2 ms the first after it, so both stay.
    click(&mut recorder, [2_100, 9_000]);
    assert_eq!(
        events_at(recorder, t(10_000)),
        [
            (0, moved(1, 0)),
            (2_000, at(1, 0)),
            (2_100, press(LEFT)),
            (2_200, at(2, 0)),
            (2_600, at(3, 0)),
            (9_000, release(LEFT)),
        ]
    );
}

#[test]
fn a_wheel_turn_read_late_inside_a_millisecond_keeps_the_position_before_it() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    report(&mut recorder, 2_000, 1, 0);
    report(&mut recorder, 2_200, 2, 0);
    report(&mut recorder, 2_300, 3, 0);
    recorder.wheel(t(2_100), 1);
    // A report 20 ms on places the three before the turn is written: the
    // turn still ends the millisecond between the first two, and the one
    // after 2.2 ms opens a millisecond of its own.
    report(&mut recorder, 22_300, 4, 0);
    click(&mut recorder, [30_000, 31_000]);
    let positions: Vec<_> = events_at(recorder, t(32_000))
        .into_iter()
        .filter(|(_, change)| matches!(change, Change::Position(_)))
        .collect();
    assert_eq!(
        positions,
        [
            (2_000, at(1, 0)),
            (2_200, at(2, 0)),
            (2_300, at(3, 0)),
            (22_300, at(4, 0)),
        ]
    );
}

#[test]
fn motion_read_late_before_the_first_position_keeps_it_at_its_time() {
    let mut recorder = positioned();
    report(&mut recorder, 1_000, 10, 20);
    // Made before the report, read after it: the report already holds it.
    recorder.motion(t(500), 1, 0);
    click(&mut recorder, [4_000, 5_000]);
    assert_eq!(
        events_at(recorder, t(6_000)),
        [
            (500, moved(1, 0)),
            (1_000, at(10, 20)),
            (4_000, press(LEFT)),
            (5_000, release(LEFT)),
        ]
    );
}

#[test]
fn positions_wait_for_late_input_twenty_milliseconds_at_most() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    report(&mut recorder, 2_000, 1, 0);
    report(&mut recorder, 2_200, 2, 0);
    // A report 20 ms on places those two: they merge.
    report(&mut recorder, 22_200, 3, 0);
    click(&mut recorder, [2_100, 30_000]);
    assert_eq!(
        events_at(recorder, t(31_000)),
        [
            (0, moved(1, 0)),
            (2_100, press(LEFT)),
            (2_200, at(2, 0)),
            (22_200, at(3, 0)),
            (30_000, release(LEFT)),
        ]
    );
}

#[test]
fn a_layout_differing_only_in_scale_keeps_positions() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    let rescaled = Layout::new(vec![
        screen([0, 0, 2048, 1280], 1000),
        screen([2048, 0, 3440, 1440], 1500),
    ])
    .unwrap();
    recorder.position(t(1_000), Point { x: 1, y: 0 }, Some(&rescaled));
    click(&mut recorder, [2_000, 3_000]);
    let recording = finish_at(recorder, t(4_000));
    assert_eq!(recording.layout(), Some(&two_screens()));
    assert_eq!(recording.events()[1].change, at(1, 0));
}

#[test]
fn the_first_position_goes_at_the_start_when_nothing_moved_before_it() {
    let mut recorder = positioned();
    recorder.key(KEYBOARD, KeyInput::press(W), t(0));
    report(&mut recorder, 3_000, 10, 20);
    recorder.motion(t(4_000), 1, 0);
    report(&mut recorder, 4_500, 11, 20);
    recorder.key(KEYBOARD, KeyInput::release(W), t(9_000));
    assert_eq!(
        events_at(recorder, t(10_000)),
        [
            (0, at(10, 20)),
            (0, press(key(W))),
            (4_000, moved(1, 0)),
            (4_500, at(11, 20)),
            (9_000, release(key(W))),
        ]
    );
}

#[test]
fn after_motion_the_first_position_goes_at_its_own_time() {
    let mut recorder = positioned();
    // Motion still in its bucket counts.
    recorder.motion(t(100), 1, 0);
    report(&mut recorder, 3_000, 10, 20);
    click(&mut recorder, [5_000, 6_000]);
    assert_eq!(
        events_at(recorder, t(7_000))[..2],
        [(100, moved(1, 0)), (3_000, at(10, 20))]
    );
}

#[test]
fn positions_are_kept_only_while_the_layout_is_the_recordings() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    let same = two_screens();
    let other = other_layout();
    recorder.position(t(1_000), Point { x: 1, y: 0 }, Some(&same));
    recorder.position(t(3_000), Point { x: 2, y: 0 }, None);
    recorder.position(t(5_000), Point { x: 3, y: 0 }, Some(&other));
    recorder.position(t(7_000), Point { x: 4, y: 0 }, Some(&same));
    click(&mut recorder, [8_000, 9_000]);
    assert_eq!(
        events_at(recorder, t(10_000)),
        [
            (0, moved(1, 0)),
            (1_000, at(1, 0)),
            (7_000, at(4, 0)),
            (8_000, press(LEFT)),
            (9_000, release(LEFT)),
        ]
    );
}

#[test]
fn a_recording_started_while_the_layout_was_unknown_has_no_positions() {
    let mut recorder = recorder();
    report(&mut recorder, 1_000, 1, 0);
    click(&mut recorder, [2_000, 3_000]);
    let recording = finish_at(recorder, t(4_000));
    assert_eq!(recording.layout(), None);
    assert_eq!(
        changes(&recording),
        [(2_000, press(LEFT)), (3_000, release(LEFT))]
    );
}

#[test]
fn a_position_on_no_screen_is_dropped() {
    let mut recorder = positioned();
    // Below the panel, inside the bounding box.
    report(&mut recorder, 1_000, 100, 1_300);
    report(&mut recorder, 2_000, 5_488, 0);
    click(&mut recorder, [3_000, 4_000]);
    let recording = finish_at(recorder, t(5_000));
    assert_eq!(recording.layout(), None);
}

#[test]
fn an_idle_recording_with_positions_saves_nothing() {
    let mut recorder = positioned();
    report(&mut recorder, 1_000, 10, 20);
    report(&mut recorder, 5_000, 11, 20);
    assert_eq!(recorder.finish(Ending::At(t(9_000))), Ok(None));
}

#[test]
fn the_recording_ends_after_its_last_position() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    report(&mut recorder, 8_000, 10, 20);
    // The stop is stamped before the position's report.
    let recording = finish_at(recorder, t(7_000));
    assert_eq!(recording.duration_us(), 8_000);
}

#[test]
fn the_tray_trip_takes_its_positions() {
    let mut recorder = positioned();
    mining_then_tray_trip(&mut recorder);
    // Positions inside the mining, and inside the trip's motion.
    report(&mut recorder, 1_700_000, 50, 50);
    report(&mut recorder, 4_050_000, 900, 10);
    report(&mut recorder, 4_150_000, 950, 5);
    report(&mut recorder, 4_700_000, 950, 1);
    let recording = tray_stop(recorder, Some(4_300_000));
    let mut expected = mining();
    expected.insert(4, (1_700_000, at(50, 50)));
    assert_eq!(changes(&recording), expected);
    assert_eq!(recording.duration_us(), 3_000_000);
    assert_eq!(recording.layout(), Some(&two_screens()));
}

#[test]
fn a_trip_that_takes_every_position_takes_the_layout_too() {
    let mut recorder = positioned();
    mining_then_tray_trip(&mut recorder);
    report(&mut recorder, 4_050_000, 900, 10);
    let recording = tray_stop(recorder, Some(4_300_000));
    assert_eq!(changes(&recording), mining());
    assert_eq!(recording.layout(), None);
}

#[test]
fn a_stop_hotkey_ends_positions_too() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    let stop = KeyInput::press(F7).firing(Action::ToggleRecording);
    recorder.key(KEYBOARD, KeyInput::press(CTRL), t(1_000));
    recorder.key(KEYBOARD, stop, t(2_000));
    report(&mut recorder, 2_500, 10, 20);
    let recording = finish_at(recorder, t(9_000));
    assert_eq!(changes(&recording), [(0, moved(1, 0))]);
    assert_eq!(recording.layout(), None);
}

#[test]
fn a_full_recording_stops_and_its_file_stays_within_its_budget_and_loads() {
    let budget = 4_000;
    let mut recorder = positioned().with_budget(budget);
    let mut now = 0;
    while !recorder.is_full() && now < 1_000_000 {
        now += 600;
        recorder.motion(t(now), 1, -1);
        report(&mut recorder, now + 300, 4_000, 1_000);
        if now % 6_000 == 0 {
            click(&mut recorder, [now + 400, now + 500]);
        }
    }
    assert!(recorder.is_full());
    let last = now;
    // Nothing more is taken.
    recorder.key(KEYBOARD, KeyInput::press(W), t(last + 10_000));
    report(&mut recorder, last + 20_000, 1, 1);
    let recording = recorder
        .finish(Ending::At(t(last + 30_000)))
        .unwrap()
        .unwrap();
    assert!(recording.duration_us() <= last + 1_000);
    let mut file = Vec::new();
    recording.write_json(&mut file).unwrap();
    let (_, events) = std::str::from_utf8(&file)
        .unwrap()
        .split_once("\"events\":[")
        .unwrap();
    assert!(events.len() as u64 <= budget, "{}", events.len());
    assert_eq!(Recording::from_json(&file).unwrap(), recording);
}

#[test]
fn the_reserve_holds_the_largest_header_layout_and_release_at_the_end() {
    let wide = screen([u32::MAX - 1, u32::MAX - 1, 1, 1], u32::MAX);
    let corner = screen([0, 0, u32::MAX, u32::MAX], u32::MAX);
    let mut outputs = vec![wide; MAX_OUTPUTS - 1];
    outputs.push(corner);
    let layout = Layout::new(outputs).unwrap();
    let everything: Vec<Held> = Key::CODES
        .chain(Button::ALL.map(Button::code))
        .filter_map(crate::recording::codes::recordable)
        .collect();
    let mut events = vec![Event {
        at: 0,
        change: at(1, 1),
    }];
    events.extend(everything.iter().map(|&held| Event {
        at: 0,
        change: press(held),
    }));
    events.extend(everything.iter().map(|&held| Event {
        at: MAX_DURATION_US,
        change: release(held),
    }));
    let name = "\u{10FFFF}".repeat(200);
    let recording =
        Recording::with_positions(name, created(), MAX_DURATION_US, Track { layout, events })
            .unwrap();
    let mut file = Vec::new();
    recording.write_json(&mut file).unwrap();
    let counted: u64 = recording
        .events()
        .iter()
        .take(1 + everything.len())
        .map(|e| event_bytes(e.change))
        .sum();
    let rest = file.len() as u64 - counted;
    assert!(rest <= FILE_RESERVE, "{rest}");
}

#[test]
fn random_input_and_positions_always_make_a_valid_recording() {
    let mut rng = fastrand::Rng::with_seed(11);
    let same = two_screens();
    let other = other_layout();
    for round in 0..300 {
        let mut recorder = positioned().with_budget(rng.u64(200..3_000));
        let mut now = 0_u64;
        for _ in 0..rng.usize(0..80) {
            now += rng.u64(0..1_500);
            let late = t(now.saturating_sub(rng.u64(0..2_000)));
            match rng.u8(0..10) {
                0..=2 => {
                    let code = [W, CTRL, F7, 0x110, 0x111][rng.usize(..5)];
                    let fired = (rng.u8(0..20) == 0).then_some(Action::ToggleRecording);
                    let input = KeyInput {
                        fired,
                        ..KeyInput::new(code, rng.bool())
                    };
                    recorder.key([KEYBOARD, MOUSE][rng.usize(..2)], input, late);
                }
                3 | 4 => recorder.motion(late, rng.i32(-5..5), rng.i32(-5..5)),
                5 => recorder.wheel(late, rng.i32(-2..2)),
                _ => {
                    let layout = [Some(&same), Some(&same), Some(&other), None][rng.usize(..4)];
                    let point = Point {
                        x: rng.u32(0..6_000),
                        y: rng.u32(0..1_500),
                    };
                    recorder.position(t(now), point, layout);
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
        let Ok(Some(recording)) = result else {
            assert!(result.is_ok(), "round {round}: {result:?}");
            continue;
        };
        let mut file = Vec::new();
        recording.write_json(&mut file).unwrap();
        assert_eq!(
            Recording::from_json(&file).as_ref(),
            Ok(&recording),
            "round {round}"
        );
    }
}

#[test]
fn a_wheel_turn_ends_the_millisecond_when_it_is_read() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    report(&mut recorder, 2_000, 1, 0);
    // The turn waits in the motion bucket; the report after it must not
    // replace the position before it.
    recorder.wheel(t(2_100), 1);
    report(&mut recorder, 2_300, 2, 0);
    report(&mut recorder, 2_400, 3, 0);
    report(&mut recorder, 2_500, 4, 0);
    assert_eq!(
        events_at(recorder, t(9_000)),
        [
            (0, moved(1, 0)),
            (2_000, at(1, 0)),
            (2_100, wheel(1)),
            (2_300, at(2, 0)),
            (2_500, at(4, 0)),
        ]
    );
}

#[test]
fn two_wheel_turns_read_before_either_is_written_end_their_own_milliseconds() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    report(&mut recorder, 4_500, 1, 0);
    recorder.wheel(t(5_000), 1);
    report(&mut recorder, 5_200, 2, 0);
    // A second turn, read while the first is still in its bucket.
    recorder.wheel(t(10_000), 1);
    click(&mut recorder, [30_000, 31_000]);
    let positions: Vec<_> = events_at(recorder, t(32_000))
        .into_iter()
        .filter(|(_, change)| matches!(change, Change::Position(_)))
        .collect();
    assert_eq!(positions, [(4_500, at(1, 0)), (5_200, at(2, 0))]);
}

#[test]
fn a_release_ends_the_millisecond() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(1_000));
    report(&mut recorder, 5_900, 1, 0);
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(6_000));
    report(&mut recorder, 6_300, 2, 0);
    let events = events_at(recorder, t(7_000));
    assert!(events.contains(&(5_900, at(1, 0))), "{events:?}");
    assert!(events.contains(&(6_300, at(2, 0))), "{events:?}");
}

#[test]
fn a_replaced_position_frees_the_bytes_it_counted() {
    let position = event_bytes(at(1, 0));
    let budget = event_bytes(moved(1, 0)) + 300 * position;
    let mut recorder = positioned().with_budget(budget);
    recorder.motion(t(0), 1, 0);
    // 400 reports over 40 ms keep about 40 positions.
    for n in 0..400 {
        report(&mut recorder, 1_000 + n * 100, 1, 0);
    }
    assert!(!recorder.is_full());
}

#[test]
fn a_chord_modifier_with_only_positions_since_is_still_left_out() {
    let mut recorder = positioned();
    recorder.key(KEYBOARD, KeyInput::press(CTRL), t(0));
    recorder.motion(t(30_000), 1, 0);
    report(&mut recorder, 31_000, 10, 20);
    // Places the first report among the written events.
    report(&mut recorder, 60_000, 11, 20);
    let stop = KeyInput::press(F7).firing(Action::ToggleRecording);
    recorder.key(KEYBOARD, stop, t(100_000));
    let recording = finish_at(recorder, t(100_000));
    let pressed = changes(&recording)
        .iter()
        .any(|(_, change)| matches!(change, Change::Press(_)));
    assert!(!pressed, "{:?}", changes(&recording));
}

#[test]
fn reports_delivered_after_two_wheel_turns_keep_both_turns_apart() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    recorder.wheel(t(1_000), 1);
    recorder.wheel(t(2_000), 1);
    // Stamped when they came, delivered after both turns were read.
    report(&mut recorder, 2_000, 1, 0);
    report(&mut recorder, 2_100, 2, 0);
    report(&mut recorder, 2_200, 3, 0);
    let positions: Vec<_> = events_at(recorder, t(9_000))
        .into_iter()
        .filter(|(_, change)| matches!(change, Change::Position(_)))
        .collect();
    assert_eq!(
        positions,
        [(2_000, at(1, 0)), (2_100, at(2, 0)), (2_200, at(3, 0))]
    );
}

#[test]
fn a_report_delivered_after_a_click_it_came_before_keeps_the_first_after_the_click() {
    let mut recorder = positioned();
    recorder.motion(t(0), 1, 0);
    recorder.key(MOUSE, KeyInput::press(button_code(LEFT)), t(2_000));
    // Stamped before the press, fed after it.
    report(&mut recorder, 1_900, 1, 0);
    report(&mut recorder, 2_100, 2, 0);
    report(&mut recorder, 2_200, 3, 0);
    recorder.key(MOUSE, KeyInput::release(button_code(LEFT)), t(9_000));
    let positions: Vec<_> = events_at(recorder, t(10_000))
        .into_iter()
        .filter(|(_, change)| matches!(change, Change::Position(_)))
        .collect();
    assert_eq!(
        positions,
        [(1_900, at(1, 0)), (2_100, at(2, 0)), (2_200, at(3, 0))]
    );
}
