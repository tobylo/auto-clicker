//! Playing positions: the plan (which moves are sent, where a position goes
//! around a click), the layout rule, and the absolute moves a run sends.

use super::*;
use crate::input::AbsolutePosition;
use crate::recording::tests::positions::screen;
use crate::recording::tests::positions::{at, two_screens};
use crate::recording::{Layout, Output, Point};
use crate::recording::{PointerSource, Track};

const MOVE: Change = Change::Move { dx: 5, dy: 0 };

fn positioned(duration_us: u64, events: &[(u64, Change)]) -> Recording {
    positioned_by(PointerSource::Kwin, duration_us, events)
}

fn positioned_by(source: PointerSource, duration_us: u64, events: &[(u64, Change)]) -> Recording {
    let events = events
        .iter()
        .map(|&(at, change)| Event { at, change })
        .collect();
    Recording::with_positions(
        "test".into(),
        created(),
        duration_us,
        Track {
            layout: two_screens(),
            source,
            events,
        },
    )
    .unwrap()
}

fn once(recording: Recording) -> Playback {
    playback(recording, 1.0, times(1))
}

/// The release that ends a fixture whose click stays down, at 90 ms.
const LAST_RELEASE: (u64, Change) = (90_000, Change::Release(LEFT));

/// What a run on the recording's own layout sends, as (time, change). A
/// fixture that leaves the button down gets [`LAST_RELEASE`], which is left
/// out of what this returns.
fn planned(events: &[(u64, Change)]) -> Vec<(u64, Change)> {
    planned_by(PointerSource::Kwin, events)
}

/// The same for a recording whose positions `source` reported.
fn planned_by(source: PointerSource, events: &[(u64, Change)]) -> Vec<(u64, Change)> {
    let presses = events
        .iter()
        .filter(|(_, c)| *c == Change::Press(LEFT))
        .count();
    let releases = events
        .iter()
        .filter(|(_, c)| *c == Change::Release(LEFT))
        .count();
    let mut events = events.to_vec();
    if presses > releases {
        events.push(LAST_RELEASE);
    }
    let playback = once(positioned_by(source, 100_000, &events));
    let motion = Motion::Positions(two_screens());
    playback
        .steps(&motion)
        .map(|e| (e.at, e.change))
        .filter(|&step| step != LAST_RELEASE)
        .collect()
}

fn output(x: u32, y: u32, width: u32, height: u32) -> Output {
    screen([x, y, width, height], 1000)
}

fn layout(outputs: Vec<Output>) -> Layout {
    Layout::new(outputs).unwrap()
}

#[test]
fn a_move_a_position_follows_within_5_ms_is_covered_and_not_sent() {
    let events = [
        (0, MOVE),
        (1_000, at(10, 10)),
        (10_000, MOVE),
        (15_000, at(20, 10)),
        (30_000, MOVE),
        (35_001, at(30, 10)),
        (50_000, MOVE),
    ];
    assert_eq!(
        planned(&events),
        [
            (1_000, at(10, 10)),
            (15_000, at(20, 10)),
            (30_000, MOVE),
            (35_001, at(30, 10)),
            (50_000, MOVE),
        ]
    );
}

#[test]
fn hyprlands_recordings_send_every_move_and_kwins_skip_the_covered() {
    let events = [
        (0, MOVE),
        (1_000, at(10, 10)),
        (1_000, MOVE),
        (10_000, MOVE),
        (11_000, Change::Press(LEFT)),
        (12_000, at(20, 10)),
    ];
    let press = (11_000, Change::Press(LEFT));
    assert_eq!(
        planned_by(PointerSource::Hyprland, &events),
        [
            (0, MOVE),
            (1_000, at(10, 10)),
            (1_000, MOVE),
            (1_000, at(10, 10)),
            (10_000, MOVE),
            (11_000, at(20, 10)),
            press,
            (12_000, at(20, 10)),
        ]
    );
    assert_eq!(
        planned_by(PointerSource::Kwin, &events),
        [(1_000, at(10, 10)), (11_000, at(20, 10)), press]
    );
}

#[test]
fn hyprlands_positions_follow_every_move_they_cover_and_go_before_each_click() {
    let press = Change::Press(LEFT);
    let release = Change::Release(LEFT);
    // A move between the click and its position: the position goes before
    // the press, and again after the move.
    let events = [
        (0, MOVE),
        (1_000, press),
        (1_500, MOVE),
        (2_000, at(10, 10)),
        (3_000, release),
    ];
    assert_eq!(
        planned_by(PointerSource::Hyprland, &events),
        [
            (0, MOVE),
            (1_000, at(10, 10)),
            (1_000, press),
            (1_500, MOVE),
            (2_000, at(10, 10)),
            (3_000, release),
        ]
    );
    // Moves around a press and a release: before each of them.
    let events = [
        (0, MOVE),
        (500, press),
        (700, MOVE),
        (900, release),
        (1_500, at(10, 10)),
    ];
    assert_eq!(
        planned_by(PointerSource::Hyprland, &events),
        [
            (0, MOVE),
            (500, at(10, 10)),
            (500, press),
            (700, MOVE),
            (900, at(10, 10)),
            (900, release),
            (1_500, at(10, 10)),
        ]
    );
    // A move read after its position at one microsecond, with or without a
    // press between them: the position again after the move.
    let events = [
        (0, MOVE),
        (2_000, at(10, 10)),
        (2_000, press),
        (2_000, MOVE),
    ];
    assert_eq!(
        planned_by(PointerSource::Hyprland, &events),
        [
            (0, MOVE),
            (2_000, at(10, 10)),
            (2_000, press),
            (2_000, MOVE),
            (2_000, at(10, 10)),
        ]
    );
}

#[test]
fn a_press_release_or_wheel_turn_is_played_after_the_position_covering_the_move_before_it() {
    for event in [Change::Press(LEFT), Change::Wheel(1)] {
        let events = [(0, MOVE), (1_000, event), (2_500, at(10, 10))];
        assert_eq!(
            planned(&events),
            [(1_000, at(10, 10)), (1_000, event)],
            "{event:?}"
        );
    }
    let events = [
        (0, Change::Press(LEFT)),
        (5_000, MOVE),
        (6_000, Change::Release(LEFT)),
        (7_900, at(10, 10)),
    ];
    assert_eq!(
        planned(&events),
        [
            (0, Change::Press(LEFT)),
            (6_000, at(10, 10)),
            (6_000, Change::Release(LEFT)),
        ]
    );
}

#[test]
fn a_position_later_than_the_delivery_delay_stays_and_the_move_is_sent() {
    let events = [(0, MOVE), (1_000, Change::Press(LEFT)), (3_001, at(10, 10))];
    assert_eq!(
        planned(&events),
        [(0, MOVE), (1_000, Change::Press(LEFT)), (3_001, at(10, 10))]
    );
    // At the delay exactly it still belongs before the press.
    let events = [(0, MOVE), (1_000, Change::Press(LEFT)), (3_000, at(10, 10))];
    assert_eq!(planned(&events)[0], (1_000, at(10, 10)));
}

#[test]
fn a_move_against_an_edge_then_a_press_then_a_move_away() {
    // The first move gives no report (the pointer is against an edge); the
    // move after the press does, and its position is the next one.
    let inside = [
        (0, MOVE),
        (1_000, Change::Press(LEFT)),
        (1_500, MOVE),
        (2_000, at(10, 10)),
    ];
    assert_eq!(
        planned(&inside),
        [(1_000, at(10, 10)), (1_000, Change::Press(LEFT))]
    );
    let outside = [
        (0, MOVE),
        (1_000, Change::Press(LEFT)),
        (3_500, MOVE),
        (4_000, at(10, 10)),
    ];
    assert_eq!(
        planned(&outside),
        [(0, MOVE), (1_000, Change::Press(LEFT)), (4_000, at(10, 10))]
    );
}

#[test]
fn a_position_goes_once_before_the_earliest_event_it_belongs_before() {
    let events = [
        (0, MOVE),
        (500, Change::Press(LEFT)),
        (700, MOVE),
        (900, Change::Release(LEFT)),
        (1_500, at(10, 10)),
    ];
    assert_eq!(
        planned(&events),
        [
            (500, at(10, 10)),
            (500, Change::Press(LEFT)),
            (900, Change::Release(LEFT)),
        ]
    );
}

#[test]
fn a_position_before_a_press_stays_before_it() {
    let events = [
        (0, MOVE),
        (500, at(10, 10)),
        (900, Change::Press(LEFT)),
        (1_200, at(11, 10)),
        (5_000, Change::Release(LEFT)),
    ];
    assert_eq!(
        planned(&events),
        [
            (500, at(10, 10)),
            (900, Change::Press(LEFT)),
            (1_200, at(11, 10)),
            (5_000, Change::Release(LEFT)),
        ]
    );
}

#[test]
fn a_move_at_the_time_of_a_position_before_it_is_covered() {
    // Read in this order at one microsecond: the report, then its move,
    // with or without a press between them.
    let events = [(0, MOVE), (2_000, at(10, 10)), (2_000, MOVE), (9_000, MOVE)];
    assert_eq!(planned(&events), [(2_000, at(10, 10)), (9_000, MOVE)]);
    let events = [
        (0, MOVE),
        (2_000, at(10, 10)),
        (2_000, Change::Press(LEFT)),
        (2_000, MOVE),
    ];
    assert_eq!(
        planned(&events),
        [(2_000, at(10, 10)), (2_000, Change::Press(LEFT))]
    );
}

#[test]
fn a_move_covered_at_its_own_time_moves_no_later_position_before_a_click() {
    // The pointer rests at A for 3 ms before the press; B belongs to the
    // move after it. Either order of A and its move gives the same plan.
    let a = at(10, 10);
    let b = at(20, 10);
    let rest = [
        (5_000, Change::Press(LEFT)),
        (5_500, MOVE),
        (6_000, b),
        (9_000, Change::Release(LEFT)),
    ];
    let expected = [
        (2_000, a),
        (5_000, Change::Press(LEFT)),
        (6_000, b),
        (9_000, Change::Release(LEFT)),
    ];
    for first in [[(2_000, a), (2_000, MOVE)], [(2_000, MOVE), (2_000, a)]] {
        let events: Vec<_> = first.iter().chain(&rest).copied().collect();
        assert_eq!(planned(&events), expected, "{first:?}");
    }
}

#[test]
fn relative_motion_sends_every_move_and_no_position() {
    let playback = once(positioned(
        100_000,
        &[
            (0, MOVE),
            (1_000, at(10, 10)),
            (2_000, Change::Press(LEFT)),
            (3_000, Change::Release(LEFT)),
        ],
    ));
    let steps: Vec<_> = playback
        .steps(&Motion::Relative)
        .map(|e| (e.at, e.change))
        .collect();
    assert_eq!(
        steps,
        [
            (0, MOVE),
            (2_000, Change::Press(LEFT)),
            (3_000, Change::Release(LEFT))
        ]
    );
}

/// A recording whose positions lie on the panel only.
fn on_the_panel() -> Playback {
    once(positioned(
        100_000,
        &[(0, at(10, 10)), (1_000, at(2_000, 1_200))],
    ))
}

#[test]
fn positions_play_while_the_screens_they_lie_on_are_unchanged() {
    let playback = on_the_panel();
    let panel = output(0, 0, 2048, 1280);
    let cases = [
        two_screens(),
        // DP-1 gone, moved below or changed: the panel is what counts.
        layout(vec![panel]),
        layout(vec![panel, output(0, 1280, 1920, 1080)]),
        layout(vec![panel, output(2048, 0, 1920, 1080)]),
        // Its scale is not compared.
        layout(vec![screen([0, 0, 2048, 1280], 2000)]),
    ];
    for current in cases {
        let expected = Motion::Positions(current.clone());
        assert_eq!(
            playback.motion(Some(&current)),
            (expected, None),
            "{current:?}"
        );
    }
}

#[test]
fn a_screen_a_position_lies_on_that_moved_resized_or_went_falls_back() {
    let playback = once(positioned(
        100_000,
        &[(0, at(10, 10)), (1_000, at(3_000, 100))],
    ));
    let cases = [
        // DP-1 left of the panel instead of right.
        layout(vec![output(0, 0, 3440, 1440), output(3440, 0, 2048, 1280)]),
        // DP-1 resized.
        layout(vec![output(0, 0, 2048, 1280), output(2048, 0, 2560, 1440)]),
        // DP-1 gone.
        layout(vec![output(0, 0, 2048, 1280)]),
    ];
    for current in cases {
        assert_eq!(
            playback.motion(Some(&current)),
            (Motion::Relative, Some(Fallback::Changed)),
            "{current:?}"
        );
    }
}

#[test]
fn an_unknown_layout_falls_back_and_a_recording_without_positions_says_nothing() {
    assert_eq!(
        on_the_panel().motion(None),
        (Motion::Relative, Some(Fallback::Unknown))
    );
    let plain = once(click());
    assert_eq!(plain.motion(None), (Motion::Relative, None));
    assert_eq!(plain.motion(Some(&two_screens())), (Motion::Relative, None));
}

#[tokio::test(start_paused = true)]
async fn a_run_sends_positions_as_absolute_moves_at_their_times() {
    let playback = once(positioned(
        10_000,
        &[
            (0, MOVE),
            (1_000, at(2_744, 720)),
            (2_000, Change::Press(LEFT)),
            (3_000, Change::Release(LEFT)),
            (4_000, MOVE),
        ],
    ));
    let (input, calls, _) = timed_input();
    let start = Instant::now();
    let motion = Motion::Positions(two_screens());
    let end = play(input, &playback, &motion, never()).await;
    assert_eq!(end.result.unwrap(), Ended::Finished);
    let calls = calls.lock().unwrap().clone();
    let sent: Vec<Call> = calls.iter().map(|(_, call)| *call).collect();
    assert_eq!(
        sent,
        [
            Call::MoveAbsolute(AbsolutePosition { x: 32768, y: 32768 }),
            LEFT_DOWN,
            LEFT_UP,
            Call::MoveRelative(5, 0),
        ]
    );
    let late = calls[0].0.checked_sub(ms(1)).unwrap();
    assert!(late <= TIMER, "{late:?} after {start:?}");
}

#[tokio::test(start_paused = true)]
async fn a_run_sends_hyprlands_positions_from_the_middle_of_their_pixel() {
    let point = Point { x: 2_744, y: 720 };
    let playback = once(positioned_by(
        PointerSource::Hyprland,
        10_000,
        &[(1_000, Change::Position(point))],
    ));
    let (input, calls, _) = timed_input();
    let motion = Motion::Positions(two_screens());
    let end = play(input, &playback, &motion, never()).await;
    assert_eq!(end.result.unwrap(), Ended::Finished);
    let sent: Vec<Call> = calls
        .lock()
        .unwrap()
        .iter()
        .map(|(_, call)| *call)
        .collect();
    let middle = two_screens().absolute_at(2_744.5, 720.5, output(2048, 0, 3440, 1440).rect());
    assert_ne!(middle, two_screens().absolute(point));
    assert_eq!(sent, [Call::MoveAbsolute(middle)]);
}

#[test]
fn the_layout_maps_the_way_the_formula_says() {
    let layout = two_screens();
    assert_eq!(
        layout.absolute(Point { x: 2_744, y: 720 }),
        AbsolutePosition { x: 32768, y: 32768 }
    );
}
