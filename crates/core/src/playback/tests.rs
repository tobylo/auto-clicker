//! Playback against the fake backend, on tokio's paused clock: every call is
//! stamped with the virtual time since the run started, so the schedule and
//! "stops at once" are checked exactly and without real waiting.

use std::future::{Future, pending, poll_fn};
use std::num::NonZeroU32;
use std::pin::pin;
use std::task::Poll;
use std::time::Duration;

use tokio::task::coop::unconstrained;
use tokio::time::{Instant, sleep, timeout};

use super::*;
use crate::input::fake::Call;
use crate::input::{Button, Held, InputError, PressState};
use crate::recording::tests::{created, key, sample};
use crate::recording::{Change, Event};
use crate::run::Ended;
use crate::test_support::{
    AnySignals, RUNAWAY, Stamped, never, shutdown_after, signals, stop_after, timed_input,
};

const W: u16 = 17;
const LEFT: Held = Held::Button(Button::Left);
const LEFT_DOWN: Call = Call::Button(Button::Left, PressState::Down);
const LEFT_UP: Call = Call::Button(Button::Left, PressState::Up);

fn us(n: u64) -> Duration {
    Duration::from_micros(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn recording(duration_us: u64, events: &[(u64, Change)]) -> Recording {
    let events = events
        .iter()
        .map(|&(at, change)| Event { at, change })
        .collect();
    Recording::new("test".to_owned(), created(), duration_us, events).unwrap()
}

fn times(count: u32) -> Repeat {
    Repeat::Times(NonZeroU32::new(count).unwrap())
}

fn playback(recording: Recording, speed: f64, repeat: Repeat) -> Playback {
    let settings = PlaybackSettings {
        speed: Speed::from_ratio(speed).unwrap(),
        repeat,
    };
    Playback::new(recording, settings).unwrap()
}

/// A left click: down at 10 ms, up at 30 ms, in a 100 ms recording.
fn click() -> Recording {
    recording(
        100_000,
        &[
            (10_000, Change::Press(LEFT)),
            (30_000, Change::Release(LEFT)),
        ],
    )
}

/// What a run did.
struct Outcome {
    result: Result<Ended, InputError>,
    /// When `play` returned, relative to its start.
    returned: Duration,
    calls: Vec<Stamped>,
    /// What the returned input still held; `None` when it was not returned.
    held: Option<Vec<Held>>,
}

impl Outcome {
    fn ended(&self) -> Ended {
        *self.result.as_ref().unwrap()
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.iter().map(|(_, call)| *call).collect()
    }

    /// When each call was made.
    fn at(&self) -> Vec<Duration> {
        self.calls.iter().map(|(at, _)| *at).collect()
    }
}

async fn play_with(playback: &Playback, signals: AnySignals) -> Outcome {
    let (input, calls, _) = timed_input();
    let start = Instant::now();
    // Every test run ends long before this; a run that never ends fails
    // here at once, since the paused clock jumps ahead.
    let end = timeout(RUNAWAY, play(input, playback, signals))
        .await
        .expect("the run never ended");
    Outcome {
        result: end.result,
        returned: start.elapsed(),
        calls: calls.lock().unwrap().clone(),
        held: end.input.map(|input| input.held().collect()),
    }
}

// The schedule

/// The timer wakes on whole milliseconds, so an event fires in the
/// millisecond after it is due.
const TIMER: Duration = Duration::from_millis(1);

#[tokio::test(start_paused = true)]
async fn a_thousand_events_fire_at_their_offsets_with_no_drift() {
    // Offsets that are never whole milliseconds: a schedule built from the
    // gaps between events, rather than from the start, would lose up to a
    // millisecond on each and end most of a second late.
    let offset = |i: u64| i * 1_337 + 211;
    let events: Vec<(u64, Change)> = (0..1000)
        .map(|i| (offset(i), Change::Move { dx: 1, dy: 0 }))
        .collect();
    let playback = playback(recording(2_000_000, &events), 1.0, times(1));

    let outcome = play_with(&playback, never()).await;

    let late: Vec<Duration> = (0..1000)
        .zip(outcome.at())
        .map(|(i, at)| at.checked_sub(us(offset(i))).expect("an event fired early"))
        .collect();
    assert_eq!(late.len(), 1000);
    assert!(late.iter().all(|late| *late < TIMER), "{late:?}");
    assert!(late[999] < TIMER, "the last event: {:?}", late[999]);
    assert_eq!(outcome.ended(), Ended::Finished);
}

#[tokio::test(start_paused = true)]
async fn the_speed_divides_every_offset() {
    for (speed, down, up) in [(1.0, 10, 30), (2.0, 5, 15), (4.0, 3, 8), (0.25, 40, 120)] {
        let outcome = play_with(&playback(click(), speed, times(1)), never()).await;
        // 2.5 and 7.5 ms at speed 4 fire on the next whole millisecond.
        assert_eq!(outcome.at(), [ms(down), ms(up)], "{speed}");
        assert_eq!(outcome.calls(), [LEFT_DOWN, LEFT_UP], "{speed}");
    }
}

#[tokio::test(start_paused = true)]
async fn every_kind_of_event_is_replayed_in_order() {
    let outcome = play_with(&playback(sample(), 1.0, times(1)), never()).await;

    let w = crate::input::Key::new(W).unwrap();
    let expected = [
        (us(0), Call::Key(w, PressState::Down)),
        (ms(2), Call::MoveRelative(3, -2)),
        (ms(2), LEFT_DOWN),
        (ms(480), Call::Scroll(-1)),
        (ms(500), LEFT_UP),
        (ms(2500), Call::Key(w, PressState::Up)),
    ];
    assert_eq!(outcome.calls, expected);
    // It ends with its last event, not after the idle time at the end.
    assert_eq!(outcome.returned, ms(2500));
    assert_eq!(outcome.held, Some(Vec::new()));
}

// Repeating

#[tokio::test(start_paused = true)]
async fn each_round_starts_a_whole_recording_after_the_one_before() {
    let outcome = play_with(&playback(click(), 1.0, times(3)), never()).await;

    let expected = [10, 30, 110, 130, 210, 230].map(ms);
    assert_eq!(outcome.at(), expected);
    assert_eq!(
        (outcome.ended(), outcome.returned),
        (Ended::Finished, ms(230))
    );
}

#[tokio::test(start_paused = true)]
async fn rounds_keep_their_length_at_any_speed() {
    let outcome = play_with(&playback(click(), 2.0, times(3)), never()).await;
    assert_eq!(outcome.at(), [5, 15, 55, 65, 105, 115].map(ms));
}

#[tokio::test(start_paused = true)]
async fn plays_until_stopped() {
    let forever = playback(click(), 1.0, Repeat::UntilStopped);
    let outcome = play_with(&forever, stop_after(ms(1005))).await;

    assert_eq!(outcome.calls.len(), 20);
    assert_eq!(
        (outcome.ended(), outcome.returned),
        (Ended::Stopped, ms(1005))
    );
}

#[tokio::test(start_paused = true)]
async fn an_empty_recording_still_takes_its_time_each_round() {
    let empty = playback(recording(100_000, &[]), 1.0, Repeat::UntilStopped);
    let outcome = play_with(&empty, stop_after(ms(350))).await;
    assert_eq!(
        (outcome.ended(), outcome.returned),
        (Ended::Stopped, ms(350))
    );
    assert_eq!(outcome.calls, []);

    let thrice = playback(recording(100_000, &[]), 1.0, times(3));
    let outcome = play_with(&thrice, never()).await;
    assert_eq!(
        (outcome.ended(), outcome.returned),
        (Ended::Finished, ms(200))
    );
}

#[test]
fn only_a_recording_of_a_tenth_of_a_second_may_repeat() {
    let cases = [
        (99_999, times(1), true),
        (0, times(1), true),
        (99_999, times(2), false),
        (99_999, Repeat::UntilStopped, false),
        (100_000, times(2), true),
        (100_000, Repeat::UntilStopped, true),
    ];
    for (duration_us, repeat, allowed) in cases {
        let settings = PlaybackSettings {
            repeat,
            ..PlaybackSettings::default()
        };
        let checked = Playback::new(recording(duration_us, &[]), settings);
        assert_eq!(checked.is_ok(), allowed, "{duration_us} {repeat:?}");
    }
}

// Stopping

#[tokio::test(start_paused = true)]
async fn a_stop_or_shutdown_between_a_down_and_its_up_releases_at_once() {
    let cases: [(fn() -> AnySignals, Ended); 3] = [
        (|| stop_after(ms(20)), Ended::Stopped),
        (|| shutdown_after(ms(20)), Ended::ShutDown),
        // Shutdown wins when both fire at the same moment.
        (|| signals(sleep(ms(20)), sleep(ms(20))), Ended::ShutDown),
    ];
    for (signals, ended) in cases {
        let outcome = play_with(&playback(click(), 1.0, times(1)), signals()).await;

        // The release comes with the signal, at 20 ms, not at the
        // recording's 30 ms; and the run is over then.
        assert_eq!(outcome.calls, [(ms(10), LEFT_DOWN), (ms(20), LEFT_UP)]);
        assert_eq!((outcome.ended(), outcome.returned), (ended, ms(20)));
        assert_eq!(outcome.held, Some(Vec::new()));
    }
}

#[tokio::test(start_paused = true)]
async fn a_signal_that_already_fired_presses_nothing() {
    let at_once = recording(
        1_000,
        &[(0, Change::Press(LEFT)), (0, Change::Release(LEFT))],
    );
    for (signals, ended) in [
        (signals(async {}, pending()), Ended::Stopped),
        (signals(pending(), async {}), Ended::ShutDown),
    ] {
        let outcome = play_with(&playback(at_once.clone(), 1.0, times(1)), signals).await;
        assert_eq!((outcome.ended(), outcome.calls()), (ended, Vec::new()));
        assert_eq!(outcome.returned, Duration::ZERO);
    }
}

#[tokio::test(start_paused = true)]
async fn a_failed_write_ends_the_run_and_releases_what_is_held() {
    let two = recording(
        100_000,
        &[
            (10_000, Change::Press(LEFT)),
            (20_000, Change::Press(key(W))),
            (30_000, Change::Release(key(W))),
            (40_000, Change::Release(LEFT)),
        ],
    );
    let (input, calls, log) = timed_input();
    let w = crate::input::Key::new(W).unwrap();
    log.fail_once(Call::Key(w, PressState::Down));

    let end = play(input, &playback(two, 1.0, times(1)), never()).await;

    assert!(
        matches!(end.result, Err(InputError::Write(_))),
        "{:?}",
        end.result
    );
    // The key whose press failed is released too: it may have gone down.
    let calls: Vec<Call> = calls
        .lock()
        .unwrap()
        .iter()
        .map(|(_, call)| *call)
        .collect();
    let released = [Call::Key(w, PressState::Up), LEFT_UP];
    assert_eq!(calls[..2], [LEFT_DOWN, Call::Key(w, PressState::Down)]);
    assert!(
        released.iter().all(|call| calls[2..].contains(call)),
        "{calls:?}"
    );
    assert_eq!(end.input.map(|input| input.held().count()), Some(0));
}

#[tokio::test(start_paused = true)]
async fn a_failed_release_at_the_end_closes_the_devices() {
    let (input, _, log) = timed_input();
    log.fail_once(LEFT_UP);
    log.fail_once(LEFT_UP);

    let end = play(input, &playback(click(), 1.0, times(1)), never()).await;

    // The recording's own release failed, and so did the one at the end.
    assert!(
        matches!(end.result, Err(InputError::Write(_))),
        "{:?}",
        end.result
    );
    assert!(end.input.is_none());
}

// Late wakes and bursts

/// Polls `future` once.
async fn poll_once<F: Future>(future: std::pin::Pin<&mut F>) -> Poll<F::Output> {
    let mut future = Some(future);
    poll_fn(|cx| Poll::Ready(future.take().expect("polled once").poll(cx))).await
}

#[tokio::test(start_paused = true)]
async fn a_late_run_fires_what_is_due_in_order_and_keeps_later_deadlines() {
    let events: Vec<(u64, Change)> = [0, 10, 20, 30, 80]
        .into_iter()
        .map(|at_ms: u64| {
            (
                at_ms * 1000,
                Change::Wheel(i32::try_from(at_ms).unwrap() + 1),
            )
        })
        .collect();
    let playback = playback(recording(100_000, &events), 1.0, times(1));
    let (input, calls, _) = timed_input();
    let mut run = pin!(play(input, &playback, never()));

    // The run starts, then is not polled again for 50 ms.
    assert!(poll_once(run.as_mut()).await.is_pending());
    sleep(ms(50)).await;
    let end = run.await;

    let expected = [
        (ms(0), Call::Scroll(1)),
        (ms(50), Call::Scroll(11)),
        (ms(50), Call::Scroll(21)),
        (ms(50), Call::Scroll(31)),
        // Not 50 ms late like the ones before it.
        (ms(80), Call::Scroll(81)),
    ];
    assert_eq!(*calls.lock().unwrap(), expected);
    assert!(matches!(end.result, Ok(Ended::Finished)));
}

#[tokio::test(start_paused = true)]
async fn many_events_at_one_moment_hand_the_thread_back_between_batches() {
    let events: Vec<(u64, Change)> = (0..1000).map(|_| (0, Change::Wheel(1))).collect();
    let playback = playback(recording(1_000, &events), 1.0, times(1));
    let (input, calls, _) = timed_input();
    // Outside tokio's own budget, which would hand the thread back as well.
    let mut run = pin!(unconstrained(play(input, &playback, never())));

    let mut fired = Vec::new();
    while poll_once(run.as_mut()).await.is_pending() {
        fired.push(calls.lock().unwrap().len());
    }

    // A batch is 256 deadlines already due. The first one holds the round's
    // own start, and the event whose turn ends a batch fires after it.
    assert_eq!(fired, [254, 510, 766]);
    assert_eq!(calls.lock().unwrap().len(), 1000);
}

#[tokio::test(start_paused = true)]
async fn a_stop_lands_between_the_batches_of_one_moment() {
    // One event, then clicks: the first batch ends on a press.
    let clicks = (0..500).flat_map(|_| [(0, Change::Press(LEFT)), (0, Change::Release(LEFT))]);
    let events: Vec<(u64, Change)> = [(0, Change::Wheel(1))].into_iter().chain(clicks).collect();
    let playback = playback(recording(1_000, &events), 1.0, times(1));
    let (input, calls, _) = timed_input();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let stopped = async {
        let _ = stopped.await;
    };
    let mut run = pin!(unconstrained(play(
        input,
        &playback,
        signals(stopped, pending())
    )));

    assert!(poll_once(run.as_mut()).await.is_pending());
    stop.send(()).unwrap();
    let end = run.await;

    // Nothing more fired after the stop but the release of the press that
    // was down.
    assert!(matches!(end.result, Ok(Ended::Stopped)), "{:?}", end.result);
    let calls = calls.lock().unwrap();
    assert_eq!(
        (calls.len(), calls[253].1, calls[254].1),
        (255, LEFT_DOWN, LEFT_UP)
    );
    assert_eq!(end.input.map(|input| input.held().count()), Some(0));
}

// Settings

#[test]
fn a_speed_is_a_number_from_a_quarter_to_four() {
    let thousandths = |ratio| Speed::from_ratio(ratio).map(Speed::thousandths);
    let cases = [
        (1.0, Some(1000)),
        (0.25, Some(250)),
        (4.0, Some(4000)),
        (1.5, Some(1500)),
        (0.3333, Some(333)),
        // Rounds to a thousandth inside the range.
        (0.2496, Some(250)),
        (4.0004, Some(4000)),
        (0.249, None),
        (4.001, None),
        (0.0, None),
        (-1.0, None),
        (f64::NAN, None),
        (f64::INFINITY, None),
        (f64::NEG_INFINITY, None),
    ];
    for (ratio, expected) in cases {
        assert_eq!(thousandths(ratio), expected, "{ratio}");
    }
}

#[test]
fn a_speed_shows_without_trailing_zeros() {
    let shown = |ratio| Speed::from_ratio(ratio).unwrap().to_string();
    assert_eq!(
        [1.0, 1.5, 0.25, 4.0, 1.05, 0.333].map(shown),
        ["1", "1.5", "0.25", "4", "1.05", "0.333"]
    );
}

#[test]
fn the_default_plays_once_at_the_recorded_speed() {
    let settings = PlaybackSettings::default();
    assert_eq!(
        (settings.speed, settings.repeat),
        (Speed::default(), times(1))
    );
    assert_eq!(Speed::default().thousandths(), 1000);
}
