//! The engine against the fake backend, on tokio's paused clock: every call is
//! stamped with the virtual time since the run started, so schedules and
//! "stops at once" are checked exactly and without real waiting.

use std::future::{Future, pending};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::{Instant, sleep, timeout};

use super::*;
use crate::input::fake::{Call, FakeBackend, FakeLog};
use crate::input::{
    AbsolutePosition, Button, Held, Input, InputBackend, InputError, Key, PressState,
};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A call and when it was made, relative to the run's start.
type Stamped = (Duration, Call);

type CallLog = Arc<Mutex<Vec<Stamped>>>;

/// The fake backend, with every call stamped with virtual time.
struct Timed {
    inner: FakeBackend,
    start: Instant,
    calls: CallLog,
}

impl Timed {
    fn note(&self, call: Call) {
        self.calls
            .lock()
            .unwrap()
            .push((self.start.elapsed(), call));
    }
}

impl InputBackend for Timed {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        self.note(Call::Button(button, state));
        self.inner.button(button, state)
    }

    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError> {
        self.note(Call::Key(key, state));
        self.inner.key(key, state)
    }

    fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        self.note(Call::MoveRelative(dx, dy));
        self.inner.move_relative(dx, dy)
    }

    fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError> {
        self.note(Call::MoveAbsolute(position));
        self.inner.move_absolute(position)
    }

    fn scroll(&mut self, notches: i32) -> Result<(), InputError> {
        self.note(Call::Scroll(notches));
        self.inner.scroll(notches)
    }
}

/// What a run did.
struct Outcome {
    result: Result<Ended, InputError>,
    /// When `run` returned, relative to its start.
    returned: Duration,
    calls: Vec<Stamped>,
    /// Whether the run handed the input back.
    input_back: bool,
    /// What the returned input still held.
    held: Vec<Held>,
}

/// The parts of an outcome most tests check, compared in one go.
#[derive(Debug, PartialEq)]
struct Summary {
    ended: Ended,
    presses: Vec<Duration>,
    releases: Vec<Duration>,
    returned: Duration,
    held: Vec<Held>,
}

impl Outcome {
    fn ended(&self) -> Ended {
        *self.result.as_ref().unwrap()
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.iter().map(|(_, call)| *call).collect()
    }

    /// When each matching call was made.
    fn times_of(&self, wanted: Call) -> Vec<Duration> {
        self.calls
            .iter()
            .filter(|(_, call)| *call == wanted)
            .map(|(at, _)| *at)
            .collect()
    }

    /// The run's end, and when `item` was pressed and released.
    fn summary(&self, (pressed, released): (Call, Call)) -> Summary {
        Summary {
            ended: self.ended(),
            presses: self.times_of(pressed),
            releases: self.times_of(released),
            returned: self.returned,
            held: self.held.clone(),
        }
    }
}

fn open() -> (Input<Timed>, CallLog, FakeLog) {
    let (inner, log) = FakeBackend::new();
    let calls = CallLog::default();
    let backend = Timed {
        inner,
        start: Instant::now(),
        calls: Arc::clone(&calls),
    };
    (Input::new(backend), calls, log)
}

/// Far longer than any run in these tests, in virtual time.
const RUNAWAY: Duration = Duration::from_secs(60 * 60);

type AnySignals = Signals<
    std::pin::Pin<Box<dyn Future<Output = ()>>>,
    std::pin::Pin<Box<dyn Future<Output = ()>>>,
>;

/// Neither signal ever fires.
fn never() -> AnySignals {
    signals(pending(), pending())
}

fn signals(
    stop: impl Future<Output = ()> + 'static,
    shutdown: impl Future<Output = ()> + 'static,
) -> AnySignals {
    Signals {
        stop: Box::pin(stop),
        shutdown: Box::pin(shutdown),
    }
}

/// Stop fires `after` the start.
fn stop_after(after: Duration) -> AnySignals {
    signals(sleep(after), pending())
}

/// Shutdown fires `after` the start.
fn shutdown_after(after: Duration) -> AnySignals {
    signals(pending(), sleep(after))
}

async fn run_with(settings: ClickerSettings, signals: AnySignals) -> Outcome {
    let (input, calls, _) = open();
    run_on(input, &calls, settings, signals).await
}

async fn run_on(
    input: Input<Timed>,
    calls: &CallLog,
    settings: ClickerSettings,
    signals: AnySignals,
) -> Outcome {
    let start = Instant::now();
    let rng = fastrand::Rng::with_seed(1);
    // Every test run ends long before this; a run that never ends (a broken
    // stop or count) fails here at once, since the paused clock jumps ahead.
    let end = timeout(RUNAWAY, run(input, &settings, rng, signals))
        .await
        .expect("the run never ended");
    Outcome {
        result: end.result,
        returned: start.elapsed(),
        calls: calls.lock().unwrap().clone(),
        input_back: end.input.is_some(),
        held: end.input.iter().flat_map(Input::held).collect(),
    }
}

fn settings(mode: Mode, interval_ms: u64) -> ClickerSettings {
    ClickerSettings {
        mode,
        interval: Interval::new(ms(interval_ms)).unwrap(),
        ..ClickerSettings::default()
    }
}

/// `settings` limited to `count` actions.
fn counted(mode: Mode, interval_ms: u64, count: u32) -> ClickerSettings {
    ClickerSettings {
        repeat: Repeat::Times(NonZeroU32::new(count).unwrap()),
        ..settings(mode, interval_ms)
    }
}

fn down(button: Button) -> Call {
    Call::Button(button, PressState::Down)
}

fn up(button: Button) -> Call {
    Call::Button(button, PressState::Up)
}

fn key_w() -> Key {
    Key::from_name("w").unwrap()
}

/// The press and release of the left button.
fn left() -> (Call, Call) {
    (down(Button::Left), up(Button::Left))
}

/// The press and release of the W key.
fn w() -> (Call, Call) {
    (
        Call::Key(key_w(), PressState::Down),
        Call::Key(key_w(), PressState::Up),
    )
}

/// `n` multiples of `step`, from `first`.
fn every(first: u64, step: u64, n: u64) -> Vec<Duration> {
    (0..n).map(|i| ms(first + i * step)).collect()
}

/// A run that ended by itself, nothing held.
fn finished(presses: Vec<Duration>, releases: Vec<Duration>, returned: Duration) -> Summary {
    Summary {
        ended: Ended::Finished,
        presses,
        releases,
        returned,
        held: Vec::new(),
    }
}

// Options

#[tokio::test(start_paused = true)]
async fn clicks_on_absolute_deadlines_for_the_repeat_count() {
    let outcome = run_with(counted(Mode::Click, 100, 100), never()).await;

    let expected = finished(every(0, 100, 100), every(10, 100, 100), ms(9910));
    assert_eq!(outcome.summary(left()), expected);
}

#[tokio::test(start_paused = true)]
async fn a_short_interval_holds_for_half_of_it() {
    let outcome = run_with(counted(Mode::Click, 12, 3), never()).await;

    let expected = finished(every(0, 12, 3), every(6, 12, 3), ms(30));
    assert_eq!(outcome.summary(left()), expected);
}

#[tokio::test(start_paused = true)]
async fn clicks_until_stopped() {
    let outcome = run_with(settings(Mode::Click, 100), stop_after(ms(1050))).await;

    let expected = Summary {
        ended: Ended::Stopped,
        ..finished(every(0, 100, 11), every(10, 100, 11), ms(1050))
    };
    assert_eq!(outcome.summary(left()), expected);
}

#[tokio::test(start_paused = true)]
async fn jitter_varies_each_interval_within_its_bounds() {
    let settings = ClickerSettings {
        jitter: Jitter::new(20).unwrap(),
        ..counted(Mode::Click, 100, 200)
    };
    let outcome = run_with(settings, never()).await;

    let presses = outcome.times_of(down(Button::Left));
    let gaps: Vec<Duration> = presses.windows(2).map(|w| w[1] - w[0]).collect();
    let within = gaps.iter().all(|gap| (ms(80)..=ms(120)).contains(gap));
    let short = gaps.iter().any(|gap| *gap < ms(90));
    let long = gaps.iter().any(|gap| *gap > ms(110));
    assert_eq!((gaps.len(), within, short, long), (199, true, true, true));
}

#[tokio::test(start_paused = true)]
async fn clicks_each_button() {
    for button in Button::ALL {
        let settings = ClickerSettings {
            button,
            ..counted(Mode::Click, 100, 1)
        };
        let outcome = run_with(settings, never()).await;
        assert_eq!(outcome.calls(), [down(button), up(button)], "{button:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn double_clicks_twice_thirty_ms_apart_inside_each_interval() {
    let settings = ClickerSettings {
        button: Button::Right,
        ..counted(Mode::DoubleClick, 100, 2)
    };
    let outcome = run_with(settings, never()).await;

    let presses = vec![ms(0), ms(30), ms(100), ms(130)];
    let releases = vec![ms(10), ms(40), ms(110), ms(140)];
    let expected = finished(presses, releases, ms(140));
    assert_eq!(
        outcome.summary((down(Button::Right), up(Button::Right))),
        expected
    );
}

#[tokio::test(start_paused = true)]
async fn hold_button_presses_once_and_releases_on_stop() {
    let outcome = run_with(settings(Mode::HoldButton, 100), stop_after(ms(5000))).await;

    let expected = [(ms(0), down(Button::Left)), (ms(5000), up(Button::Left))];
    assert_eq!(
        (outcome.ended(), outcome.calls),
        (Ended::Stopped, expected.to_vec())
    );
}

#[tokio::test(start_paused = true)]
async fn hold_key_presses_once_and_releases_on_stop() {
    // A hold ignores the repeat count and the fixed point.
    let settings = ClickerSettings {
        key: key_w(),
        position: Position::Fixed(AbsolutePosition { x: 1, y: 2 }),
        ..counted(Mode::HoldKey, 100, 1)
    };
    let outcome = run_with(settings, stop_after(ms(5000))).await;

    let expected = [(ms(0), w().0), (ms(5000), w().1)];
    assert_eq!(outcome.calls, expected);
}

#[tokio::test(start_paused = true)]
async fn key_repeat_taps_the_key_each_interval() {
    let settings = ClickerSettings {
        key: key_w(),
        ..counted(Mode::KeyRepeat, 50, 3)
    };
    let outcome = run_with(settings, never()).await;

    let expected = finished(every(0, 50, 3), every(10, 50, 3), ms(110));
    assert_eq!(outcome.summary(w()), expected);
    assert_eq!(outcome.calls.len(), 6, "only the key was pressed");
}

#[tokio::test(start_paused = true)]
async fn a_fixed_point_moves_there_before_each_action() {
    let point = AbsolutePosition { x: 100, y: 65535 };
    let moved = Call::MoveAbsolute(point);
    let fixed = |mode| ClickerSettings {
        position: Position::Fixed(point),
        ..counted(mode, 100, 2)
    };
    let (press, release) = left();

    let clicks = run_with(fixed(Mode::Click), never()).await;
    let one_click = [moved, press, release];
    assert_eq!(clicks.calls(), [one_click, one_click].concat());

    let double = run_with(fixed(Mode::DoubleClick), never()).await;
    assert_eq!(double.times_of(moved), [ms(0), ms(100)]);

    let hold = run_with(fixed(Mode::HoldButton), stop_after(ms(50))).await;
    assert_eq!(hold.calls(), one_click);
}

#[tokio::test(start_paused = true)]
async fn at_the_cursor_and_for_keys_the_pointer_never_moves() {
    let at_cursor = run_with(counted(Mode::Click, 100, 2), never()).await;
    let keys = ClickerSettings {
        position: Position::Fixed(AbsolutePosition { x: 5, y: 5 }),
        ..counted(Mode::KeyRepeat, 100, 2)
    };
    let keys = run_with(keys, never()).await;

    for outcome in [at_cursor, keys] {
        let moves = outcome
            .calls()
            .into_iter()
            .filter(|call| matches!(call, Call::MoveAbsolute(_) | Call::MoveRelative(..)));
        assert_eq!(moves.count(), 0);
    }
}

// Stopping and shutting down

/// A stop or shutdown must end the run within this much virtual time: far
/// under the shortest wait it interrupts (a 5 ms remainder of a hold), so a
/// run that ignores the signal and finishes the wait fails.
const AT_ONCE: Duration = Duration::from_millis(1);

/// A signal that arrives while something is pressed.
struct MidPress {
    name: &'static str,
    settings: ClickerSettings,
    /// When the signal fires.
    after: Duration,
    /// The press and release of what is held at that moment.
    item: (Call, Call),
}

/// The waits a signal can interrupt mid-press, and a time inside each.
fn mid_press_cases() -> [MidPress; 5] {
    let with_w = |mode| ClickerSettings {
        key: key_w(),
        ..settings(mode, 100)
    };
    let case = |name, settings, after, item| MidPress {
        name,
        settings,
        after: ms(after),
        item,
    };
    [
        case("hold button", settings(Mode::HoldButton, 100), 5000, left()),
        case("hold key", with_w(Mode::HoldKey), 5000, w()),
        case(
            "between down and up",
            settings(Mode::Click, 100),
            105,
            left(),
        ),
        case(
            "second click of a double",
            settings(Mode::DoubleClick, 100),
            135,
            left(),
        ),
        case("between key down and up", with_w(Mode::KeyRepeat), 5, w()),
    ]
}

/// Asserts the run ended by `signal` at once, with the case's item pressed
/// before the signal and released after it, and nothing left held.
fn assert_interrupted(case: &MidPress, outcome: &Outcome, signal: Ended) {
    let (pressed, released) = case.item;
    let before = |call| {
        outcome
            .times_of(call)
            .into_iter()
            .filter(|at| *at < case.after)
            .max()
    };
    // The press under test came before the signal and was not yet released.
    let pressed_and_held = before(pressed) > before(released);
    let at_once = outcome.returned - case.after < AT_ONCE;
    let actual = (
        outcome.ended(),
        pressed_and_held,
        at_once,
        outcome.calls.last(),
        &outcome.held,
    );
    let expected = (
        signal,
        true,
        true,
        Some(&(outcome.returned, released)),
        &Vec::new(),
    );
    assert_eq!(actual, expected, "{}", case.name);
}

#[tokio::test(start_paused = true)]
async fn stop_mid_press_releases_at_once() {
    for case in mid_press_cases() {
        let outcome = run_with(case.settings, stop_after(case.after)).await;
        assert_interrupted(&case, &outcome, Ended::Stopped);
    }
}

#[tokio::test(start_paused = true)]
async fn shutdown_mid_press_releases_at_once() {
    for case in mid_press_cases() {
        let outcome = run_with(case.settings, shutdown_after(case.after)).await;
        assert_interrupted(&case, &outcome, Ended::ShutDown);
    }
}

/// A run of `settings` stopped, and one shut down, `after` the start.
async fn both_signals_after(settings: ClickerSettings, after: Duration) -> [(Ended, Outcome); 2] {
    [
        (Ended::Stopped, run_with(settings, stop_after(after)).await),
        (
            Ended::ShutDown,
            run_with(settings, shutdown_after(after)).await,
        ),
    ]
}

#[tokio::test(start_paused = true)]
async fn stop_between_the_clicks_of_a_double_click_ends_at_once() {
    // At 20 ms the first click is released and the second is due at 30 ms.
    for (signal, outcome) in both_signals_after(settings(Mode::DoubleClick, 100), ms(20)).await {
        let expected = Summary {
            ended: signal,
            ..finished(vec![ms(0)], vec![ms(10)], ms(20))
        };
        assert_eq!(outcome.summary(left()), expected);
    }
}

#[tokio::test(start_paused = true)]
async fn stop_between_actions_ends_at_once() {
    for (signal, outcome) in both_signals_after(settings(Mode::Click, 1000), ms(500)).await {
        let expected = Summary {
            ended: signal,
            ..finished(vec![ms(0)], vec![ms(10)], ms(500))
        };
        assert_eq!(outcome.summary(left()), expected);
    }
}

#[tokio::test(start_paused = true)]
async fn a_signal_that_already_fired_prevents_the_first_action() {
    for mode in [Mode::Click, Mode::HoldButton] {
        let stopped = run_with(settings(mode, 100), signals(async {}, pending())).await;
        let shut_down = run_with(settings(mode, 100), signals(pending(), async {})).await;
        let actual = [
            (stopped.ended(), stopped.calls),
            (shut_down.ended(), shut_down.calls),
        ];
        let expected = [(Ended::Stopped, Vec::new()), (Ended::ShutDown, Vec::new())];
        assert_eq!(actual, expected, "{mode:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn shutdown_wins_over_a_stop_at_the_same_moment() {
    // While holding, during a click's hold, and between clicks.
    let cases = [
        (Mode::HoldButton, 50),
        (Mode::Click, 105),
        (Mode::Click, 150),
    ];
    for (mode, after) in cases {
        let at_once = signals(sleep(ms(after)), sleep(ms(after)));
        let outcome = run_with(settings(mode, 100), at_once).await;
        assert_eq!(outcome.ended(), Ended::ShutDown, "{mode:?} at {after} ms");
    }
}

#[tokio::test(start_paused = true)]
async fn dropping_the_run_mid_press_releases() {
    let (input, calls, _) = open();
    let settings = settings(Mode::HoldButton, 100);
    let rng = fastrand::Rng::with_seed(1);

    let dropped = timeout(ms(50), run(input, &settings, rng, never())).await;

    let expected = vec![(ms(0), down(Button::Left)), (ms(50), up(Button::Left))];
    let actual = (dropped.is_err(), calls.lock().unwrap().clone());
    assert_eq!(actual, (true, expected));
}

// Errors

/// A run of `settings` in which the first attempt of `failing` fails.
async fn run_failing(failing: Call, settings: ClickerSettings, signals: AnySignals) -> Outcome {
    let (input, calls, log) = open();
    log.fail_once(failing);
    run_on(input, &calls, settings, signals).await
}

fn write_error(outcome: &Outcome) -> bool {
    matches!(outcome.result, Err(InputError::Write(_)))
}

#[tokio::test(start_paused = true)]
async fn a_failed_press_ends_the_run_and_still_releases() {
    // A count, so a run that ignores the error ends instead of clicking on.
    let outcome = run_failing(down(Button::Left), counted(Mode::Click, 100, 3), never()).await;

    let expected = vec![(ms(0), down(Button::Left)), (ms(0), up(Button::Left))];
    let actual = (write_error(&outcome), outcome.calls, outcome.held);
    assert_eq!(actual, (true, expected, Vec::new()));
}

#[tokio::test(start_paused = true)]
async fn a_failed_release_is_reported_and_retried_on_the_way_out() {
    let outcome = run_failing(up(Button::Left), counted(Mode::Click, 100, 3), never()).await;

    let (press, release) = left();
    let expected = vec![(ms(0), press), (ms(10), release), (ms(10), release)];
    let actual = (write_error(&outcome), outcome.calls, outcome.held);
    assert_eq!(actual, (true, expected, Vec::new()));
}

#[tokio::test(start_paused = true)]
async fn the_run_reports_its_own_error_before_a_failed_release() {
    let (input, calls, log) = open();
    log.fail_once(down(Button::Left));
    log.fail_once(up(Button::Left));

    let outcome = run_on(input, &calls, counted(Mode::Click, 100, 3), never()).await;

    let message = outcome.result.map_err(|err| err.to_string()).unwrap_err();
    assert!(message.contains("Down"), "{message}");
}

#[tokio::test(start_paused = true)]
async fn a_failed_release_after_a_stop_is_reported() {
    let hold = settings(Mode::HoldButton, 100);
    let outcome = run_failing(up(Button::Left), hold, stop_after(ms(50))).await;

    // The run drops the input it could not clean, and the drop tries again.
    let releases = outcome.times_of(up(Button::Left));
    let actual = (write_error(&outcome), releases, outcome.input_back);
    assert_eq!(actual, (true, vec![ms(50), ms(50)], false));
}

#[tokio::test(start_paused = true)]
async fn a_clean_run_hands_the_input_back_with_nothing_held() {
    let finished = run_with(counted(Mode::Click, 100, 1), never()).await;
    let stopped = run_with(settings(Mode::HoldButton, 100), stop_after(ms(50))).await;
    for outcome in [finished, stopped] {
        assert_eq!((outcome.input_back, outcome.held), (true, Vec::new()));
    }
}
