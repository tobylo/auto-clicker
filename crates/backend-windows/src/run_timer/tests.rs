//! The run timer on Windows itself: no input is sent, so these run in CI's
//! Windows job and in the test VM alike.

use std::future::{Future, pending, poll_fn};
use std::num::NonZeroU32;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, MutexGuard};

use auto_clicker_core::clicker::{ClickerSettings, Interval, Mode, Repeat, run};
use auto_clicker_core::input::fake::{Call, FakeBackend, FakeLog};
use auto_clicker_core::input::{AbsolutePosition, Button, Input, InputBackend, Key, PressState};
use auto_clicker_core::run::{Ended, Signals};
use tokio::time::{sleep, timeout};

use super::*;

/// Run timer threads alive in this process. Counted by the threads
/// themselves, in tests only: a thread a timer's drop left behind stays
/// counted.
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// Counts its thread as alive until the thread's last act.
pub(super) struct Live(());

impl Live {
    pub(super) fn new() -> Live {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Live(())
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Taken by every test that starts a timer, so no other test's thread or
/// opt-out is counted in its checks.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn one_at_a_time() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Nothing of a run is left: no timer thread, Windows manages throttling
/// again, and the display is kept on only if something else asks for it.
fn assert_idle(case: &str) {
    assert_eq!(
        LIVE.load(Ordering::SeqCst),
        0,
        "{case}: a timer thread is left"
    );
    assert!(
        execution_speed_managed().unwrap(),
        "{case}: still opted out"
    );
    assert!(
        !display_kept_on().unwrap(),
        "{case}: the display is still kept on"
    );
}

async fn wait(timer: &mut WaitableTimer, deadline: tokio::time::Instant) {
    poll_fn(|cx| timer.poll_until(cx, deadline)).await.unwrap();
}

#[test]
fn a_due_time_is_relative_in_100_ns_units_and_never_zero() {
    assert_eq!(relative_due_time(Duration::ZERO), -1);
    assert_eq!(relative_due_time(Duration::from_nanos(99)), -1);
    assert_eq!(relative_due_time(ms(10)), -100_000);
    assert_eq!(relative_due_time(Duration::MAX), -i64::MAX);
}

#[test]
fn waits_until_each_deadline_and_not_before() {
    let _serial = one_at_a_time();
    runtime().block_on(async {
        let mut timer = WaitableTimer::start().unwrap();
        let start = tokio::time::Instant::now();
        for step in 1..=10 {
            let deadline = start + ms(10 * step);
            wait(&mut timer, deadline).await;
            let woke = tokio::time::Instant::now();
            assert!(woke >= deadline, "woke {:?} early", deadline - woke);
            // Generous: a slow runner may wake late, never by this much.
            assert!(woke < deadline + ms(500), "woke {:?} late", woke - deadline);
        }
    });
    assert_idle("after the waits");
}

#[test]
fn only_the_latest_deadline_completes_a_wait() {
    let _serial = one_at_a_time();
    runtime().block_on(async {
        let mut timer = WaitableTimer::start().unwrap();
        let start = tokio::time::Instant::now();
        let near = start + ms(20);
        let far = start + ms(400);
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        // Asked for the near deadline, then moved on to the far one.
        assert!(timer.poll_until(&mut cx, near).is_pending());
        assert!(timer.poll_until(&mut cx, far).is_pending());
        sleep(ms(100)).await;
        // The near deadline has passed, but the wait is for the far one.
        assert!(timer.poll_until(&mut cx, far).is_pending());
        wait(&mut timer, far).await;
        assert!(tokio::time::Instant::now() >= far);
    });
}

#[test]
fn a_deadline_already_past_is_ready_at_once() {
    let _serial = one_at_a_time();
    let mut timer = WaitableTimer::start().unwrap();
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let past = tokio::time::Instant::now();
    thread::sleep(ms(1));
    assert!(timer.poll_until(&mut cx, past).is_ready());
}

// Every way a run ends

/// The fake output with this crate's run timer, noting at each press
/// whether the run had opted out of throttling and kept the display on, and
/// panicking at a press when told to.
struct Probe {
    inner: FakeBackend,
    during: Arc<Mutex<Vec<During>>>,
    panic_at_press: bool,
}

/// What held at a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct During {
    opted_out: bool,
    display_on: bool,
}

const ACTIVE: During = During {
    opted_out: true,
    display_on: true,
};

impl InputBackend for Probe {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        if state == PressState::Down {
            let during = During {
                opted_out: !execution_speed_managed().unwrap(),
                display_on: display_kept_on().unwrap(),
            };
            self.during.lock().unwrap().push(during);
            assert!(!self.panic_at_press, "a press that panics");
        }
        self.inner.button(button, state)
    }

    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError> {
        self.inner.key(key, state)
    }

    fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        self.inner.move_relative(dx, dy)
    }

    fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError> {
        self.inner.move_absolute(position)
    }

    fn scroll(&mut self, notches: i32) -> Result<(), InputError> {
        self.inner.scroll(notches)
    }

    fn run_timer(&mut self) -> Result<Box<dyn RunTimer>, InputError> {
        WaitableTimer::start()
            .map(|timer| Box::new(timer) as Box<dyn RunTimer>)
            .map_err(InputError::Timer)
    }
}

struct Opened {
    input: Input<Probe>,
    log: FakeLog,
    during: Arc<Mutex<Vec<During>>>,
}

fn open(panic_at_press: bool) -> Opened {
    let (inner, log) = FakeBackend::new();
    let during = Arc::default();
    let probe = Probe {
        inner,
        during: Arc::clone(&during),
        panic_at_press,
    };
    Opened {
        input: Input::new(probe),
        log,
        during,
    }
}

fn clicks(interval: Duration, count: Option<u32>) -> ClickerSettings {
    ClickerSettings {
        mode: Mode::Click,
        interval: Interval::new(interval).unwrap(),
        repeat: count.map_or(Repeat::UntilStopped, |n| {
            Repeat::Times(NonZeroU32::new(n).unwrap())
        }),
        ..ClickerSettings::default()
    }
}

type Signal = Pin<Box<dyn Future<Output = ()>>>;

fn signals(stop: Signal, shutdown: Signal) -> Signals<Signal, Signal> {
    Signals { stop, shutdown }
}

fn never() -> Signal {
    Box::pin(pending())
}

/// Fires `wait` after the run first polls it, on the run's runtime.
fn after(wait: Duration) -> Signal {
    Box::pin(async move { sleep(wait).await })
}

/// What a case's run did.
struct Ran {
    /// How the run ended; `None` when it panicked or was dropped.
    result: Option<Result<Ended, InputError>>,
    calls: Vec<Call>,
    during: Vec<During>,
    took: Duration,
}

fn run_case(
    opened: Opened,
    settings: &ClickerSettings,
    signals: Signals<Signal, Signal>,
    drop_after: Option<Duration>,
) -> Ran {
    let Opened { input, log, during } = opened;
    let start = Instant::now();
    let rng = fastrand::Rng::with_seed(1);
    let result = catch_unwind(AssertUnwindSafe(|| {
        runtime().block_on(async {
            let running = run(input, settings, rng, signals);
            match drop_after {
                Some(wait) => timeout(wait, running).await.ok().map(|end| end.result),
                None => Some(running.await.result),
            }
        })
    }))
    .ok()
    .flatten();
    let took = start.elapsed();
    let during = during.lock().unwrap().clone();
    Ran {
        result,
        calls: log.calls(),
        during,
        took,
    }
}

/// One way for a run to end.
struct Case {
    name: &'static str,
    opened: Opened,
    settings: ClickerSettings,
    signals: Signals<Signal, Signal>,
    /// Drops the run this long after its start.
    drop_after: Option<Duration>,
    /// How it ends; `None` for an error, a panic or a drop.
    ended: Option<Ended>,
}

fn cases() -> [Case; 6] {
    let slow = clicks(Duration::from_secs(10), None);
    let failing = open(false);
    failing
        .log
        .fail_once(Call::Button(Button::Left, PressState::Down));
    let case = |name, opened, settings, signals, ended| Case {
        name,
        opened,
        settings,
        signals,
        drop_after: None,
        ended,
    };
    [
        case(
            "finished",
            open(false),
            clicks(ms(10), Some(5)),
            signals(never(), never()),
            Some(Ended::Finished),
        ),
        case(
            "stopped",
            open(false),
            slow,
            signals(after(ms(50)), never()),
            Some(Ended::Stopped),
        ),
        case(
            "shut down",
            open(false),
            slow,
            signals(never(), after(ms(50))),
            Some(Ended::ShutDown),
        ),
        case(
            "write error",
            failing,
            slow,
            signals(never(), never()),
            None,
        ),
        case(
            "panicked",
            open(true),
            slow,
            signals(never(), never()),
            None,
        ),
        Case {
            drop_after: Some(ms(50)),
            ..case(
                "dropped",
                open(false),
                slow,
                signals(never(), never()),
                None,
            )
        },
    ]
}

#[test]
fn every_way_a_run_ends_leaves_no_thread_and_hands_throttling_back() {
    let _serial = one_at_a_time();
    assert_idle("before the runs");
    for case in cases() {
        let name = case.name;
        let ran = run_case(case.opened, &case.settings, case.signals, case.drop_after);
        assert_idle(name);
        // Opted out and the display kept on during the run, at every press.
        assert!(!ran.during.is_empty(), "{name}: nothing pressed");
        assert!(
            ran.during.iter().all(|&during| during == ACTIVE),
            "{name}: {:?}",
            ran.during
        );
        // Whatever ended it, the last press was released.
        let released = Call::Button(Button::Left, PressState::Up);
        assert_eq!(ran.calls.last(), Some(&released), "{name}");
        // Stop, shutdown and the drop end the 10 s wait at once.
        assert!(
            ran.took < Duration::from_secs(1),
            "{name}: took {:?}",
            ran.took
        );
        let ended = ran
            .result
            .as_ref()
            .and_then(|result| result.as_ref().ok().copied());
        assert_eq!(ended, case.ended, "{name}");
    }
}
