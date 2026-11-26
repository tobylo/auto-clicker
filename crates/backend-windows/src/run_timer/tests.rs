//! The run timer on Windows itself: no input is sent, so these run in CI's
//! Windows job and in the test VM alike.

use std::future::{Future, pending, poll_fn};
use std::num::NonZeroU32;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, MutexGuard};
use std::task::Wake;

use auto_clicker_core::clicker::{ClickerSettings, Interval, Mode, Repeat, run};
use auto_clicker_core::input::fake::{Call, FakeBackend, FakeLog};
use auto_clicker_core::input::{AbsolutePosition, Button, Input, InputBackend, Key, PressState};
use auto_clicker_core::run::{Ended, Signals};
use tokio::time::sleep;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

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

/// Run timer threads alive now.
pub(crate) fn live() -> usize {
    LIVE.load(Ordering::SeqCst)
}

/// How long a timer thread waits before it asks to keep the display on and
/// is ready, in ms, unless told to stop; 0 in all but the tests that set it.
static SLOW_START: AtomicU64 = AtomicU64::new(0);

/// Called by each timer thread as it starts: waits [`SLOW_START`], ending
/// at once on `stop`.
#[allow(unsafe_code)]
pub(super) fn slow_start(stop: &OwnedHandle) {
    let wait = u32::try_from(SLOW_START.load(Ordering::SeqCst)).unwrap();
    if wait > 0 {
        // SAFETY: an open event handle, alive for the call.
        unsafe { WaitForSingleObject(stop.as_raw_handle(), wait) };
    }
}

/// Makes timer threads fail once ready and the run has pressed, as a
/// failed wait or arm would part way through a hold.
static BROKEN: AtomicBool = AtomicBool::new(false);

/// Whether a [`Probe`] has pressed since the last [`Broken`] was set.
static PRESSED: AtomicBool = AtomicBool::new(false);

/// Called by each timer thread once ready: fails once the run has pressed
/// (within 5 s) while [`BROKEN`] is set.
pub(super) fn maybe_break() -> io::Result<()> {
    if !BROKEN.load(Ordering::SeqCst) {
        return Ok(());
    }
    let until = Instant::now() + Duration::from_secs(5);
    while !PRESSED.load(Ordering::SeqCst) && Instant::now() < until {
        thread::sleep(ms(1));
    }
    Err(io::Error::other("broken on purpose"))
}

/// Sets [`SLOW_START`] until dropped, a failed test included.
struct SlowStart;

impl SlowStart {
    fn set(wait_ms: u64) -> SlowStart {
        SLOW_START.store(wait_ms, Ordering::SeqCst);
        SlowStart
    }
}

impl Drop for SlowStart {
    fn drop(&mut self) {
        SLOW_START.store(0, Ordering::SeqCst);
    }
}

/// Sets [`BROKEN`] until dropped, a failed test included.
struct Broken;

impl Broken {
    fn set() -> Broken {
        PRESSED.store(false, Ordering::SeqCst);
        BROKEN.store(true, Ordering::SeqCst);
        Broken
    }
}

impl Drop for Broken {
    fn drop(&mut self) {
        BROKEN.store(false, Ordering::SeqCst);
    }
}

/// Taken by every test that starts a timer, so no other test's thread or
/// opt-out is counted in its checks.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

pub(crate) fn one_at_a_time() -> MutexGuard<'static, ()> {
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

/// Whether the display state can tell this program's request apart:
/// Windows reports one state for every process, so while another program
/// keeps the display on (a video playing, say), the checks on it are left
/// out.
fn display_is_ours() -> bool {
    let ours = !display_kept_on().unwrap();
    if !ours {
        eprintln!("another program keeps the display on: its checks are left out");
    }
    ours
}

/// Nothing of a run is left: no timer thread, Windows manages throttling
/// again, and, when `display` says the state is ours to check, the display
/// is no longer kept on.
fn assert_idle(case: &str, display: bool) {
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
        !display || !display_kept_on().unwrap(),
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
    let display = display_is_ours();
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
    assert_idle("after the waits", display);
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
    runtime().block_on(async {
        let mut timer = WaitableTimer::start().unwrap();
        // The first wait waits for the thread.
        wait(&mut timer, tokio::time::Instant::now()).await;
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let past = tokio::time::Instant::now();
        thread::sleep(ms(1));
        assert!(timer.poll_until(&mut cx, past).is_ready());
    });
}

/// Counts its wakes.
#[derive(Default)]
struct Wakes(AtomicUsize);

impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn no_wait_completes_before_the_thread_is_ready() {
    let _serial = one_at_a_time();
    // Reset only once the thread is past it: it reads it as it starts.
    let _slow = SlowStart::set(300);
    runtime().block_on(async {
        let mut timer = WaitableTimer::start().unwrap();
        let wakes = Arc::new(Wakes::default());
        let waker = Arc::clone(&wakes).into();
        let mut cx = Context::from_waker(&waker);
        let past = tokio::time::Instant::now();
        // Already due, but the thread has not asked for the display yet.
        assert!(timer.poll_until(&mut cx, past).is_pending());
        let until = Instant::now() + Duration::from_secs(5);
        while wakes.0.load(Ordering::SeqCst) == 0 && Instant::now() < until {
            thread::sleep(ms(10));
        }
        // Woken once ready, and ready then.
        assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
        assert!(timer.poll_until(&mut cx, past).is_ready());
    });
}

#[test]
fn a_thread_not_ready_within_a_second_fails_the_run_with_nothing_pressed() {
    let _serial = one_at_a_time();
    // Ready after 5 s unless stopped first, as the run's end does.
    let _slow = SlowStart::set(5_000);
    let slow = clicks(Duration::from_secs(10), None);
    let ran = run_case(open(false), &slow, signals(never(), never()), false);

    let failed = matches!(&ran.exit, Exit::Failed(InputError::Timer(err)) if err.kind() == io::ErrorKind::TimedOut);
    assert!(failed, "{:?}", ran.exit);
    assert_eq!(ran.calls, Vec::new());
    let took = ran.took;
    assert!(took >= JOIN_WITHIN && took < ms(3_000), "took {took:?}");
}

#[test]
fn a_failed_thread_ends_a_wait_for_a_deadline_with_the_error() {
    let _serial = one_at_a_time();
    runtime().block_on(async {
        let mut timer = WaitableTimer::start().unwrap();
        wait(&mut timer, tokio::time::Instant::now()).await;
        fail(&timer.shared, (io::ErrorKind::Other, "broken".to_owned()));
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let ahead = tokio::time::Instant::now() + Duration::from_secs(10);
        let failed = timer.poll_until(&mut cx, ahead);
        assert!(
            matches!(&failed, Poll::Ready(Err(InputError::Timer(err))) if err.to_string() == "broken"),
            "{failed:?}"
        );
    });
}

#[test]
fn a_wake_for_a_deadline_the_run_moved_on_from_is_not_passed_on() {
    let _serial = one_at_a_time();
    runtime().block_on(async {
        let mut timer = WaitableTimer::start().unwrap();
        wait(&mut timer, tokio::time::Instant::now()).await;
        let wakes = Arc::new(Wakes::default());
        let waker = Arc::clone(&wakes).into();
        let mut cx = Context::from_waker(&waker);
        let start = tokio::time::Instant::now();
        let (first, second) = (start + ms(5_000), start + ms(6_000));
        assert!(timer.poll_until(&mut cx, first).is_pending());
        let moved_from = timer.shared.lock().request.unwrap();
        assert!(timer.poll_until(&mut cx, second).is_pending());
        fire(&timer.shared, moved_from);
        assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
        assert!(timer.poll_until(&mut cx, second).is_pending());
    });
}

#[test]
fn a_thread_that_fails_while_serving_ends_a_hold_and_lets_go() {
    let _serial = one_at_a_time();
    let _broken = Broken::set();
    let hold = ClickerSettings {
        mode: Mode::HoldButton,
        ..ClickerSettings::default()
    };
    // The stop only bounds the test.
    let ran = run_case(
        open(false),
        &hold,
        signals(after(ms(5_000)), never()),
        false,
    );

    assert!(
        matches!(ran.exit, Exit::Failed(InputError::Timer(_))),
        "{:?}",
        ran.exit
    );
    let pressed = vec![
        Call::Button(Button::Left, PressState::Down),
        Call::Button(Button::Left, PressState::Up),
    ];
    assert_eq!(ran.calls, pressed);
    assert!(ran.took < Duration::from_secs(1), "took {:?}", ran.took);
}

#[test]
fn a_failed_thread_wakes_a_wait_without_a_deadline_with_the_error() {
    let _serial = one_at_a_time();
    runtime().block_on(async {
        let mut timer = WaitableTimer::start().unwrap();
        wait(&mut timer, tokio::time::Instant::now()).await;
        let wakes = Arc::new(Wakes::default());
        let waker = Arc::clone(&wakes).into();
        let mut cx = Context::from_waker(&waker);
        assert!(timer.poll_failed(&mut cx).is_pending());
        fail(&timer.shared, (io::ErrorKind::Other, "broken".to_owned()));
        assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
        let failed = timer.poll_failed(&mut cx);
        assert!(
            matches!(&failed, Poll::Ready(InputError::Timer(err)) if err.to_string() == "broken"),
            "{failed:?}"
        );
    });
}

// Every way a run ends

/// The fake output with this crate's run timer, noting at each press
/// whether the run had opted out of throttling and kept the display on, and
/// panicking at a press when told to. Every press sets [`PRESSED`].
struct Probe {
    inner: FakeBackend,
    during: Presses,
    panic_at_press: bool,
}

/// What held at each press so far.
type Presses = Arc<Mutex<Vec<During>>>;

/// What held at a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct During {
    opted_out: bool,
    display_on: bool,
}

impl InputBackend for Probe {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        if state == PressState::Down {
            let during = During {
                opted_out: execution_speed_opted_out().unwrap(),
                display_on: display_kept_on().unwrap(),
            };
            self.during.lock().unwrap().push(during);
            PRESSED.store(true, Ordering::SeqCst);
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
    during: Presses,
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

/// Fires once the run has pressed, as `during` records it.
async fn pressed(during: Presses) {
    while during.lock().unwrap().is_empty() {
        sleep(ms(1)).await;
    }
}

/// Fires once the run has pressed.
fn after_the_press(during: &Presses) -> Signal {
    Box::pin(pressed(Arc::clone(during)))
}

/// How a case's run ended.
#[derive(Debug)]
enum Exit {
    Ended(Ended),
    Failed(InputError),
    Panicked,
    Dropped,
}

/// What a case's run did.
struct Ran {
    exit: Exit,
    calls: Vec<Call>,
    during: Vec<During>,
    took: Duration,
}

/// Runs `opened` with `settings` until it ends, or, with
/// `drop_once_pressed`, drops it once it has pressed.
fn run_case(
    opened: Opened,
    settings: &ClickerSettings,
    signals: Signals<Signal, Signal>,
    drop_once_pressed: bool,
) -> Ran {
    let Opened { input, log, during } = opened;
    let start = Instant::now();
    let rng = fastrand::Rng::with_seed(1);
    let watched = Arc::clone(&during);
    let exit = catch_unwind(AssertUnwindSafe(|| {
        runtime().block_on(async {
            let running = run(input, settings, rng, signals);
            let dropping = async {
                if drop_once_pressed {
                    pressed(watched).await;
                } else {
                    pending::<()>().await;
                }
            };
            tokio::select! {
                end = running => match end.result {
                    Ok(ended) => Exit::Ended(ended),
                    Err(err) => Exit::Failed(err),
                },
                () = dropping => Exit::Dropped,
            }
        })
    }))
    .unwrap_or(Exit::Panicked);
    let took = start.elapsed();
    let during = during.lock().unwrap().clone();
    Ran {
        exit,
        calls: log.calls(),
        during,
        took,
    }
}

/// How a case is to end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    Ended(Ended),
    WriteError,
    Panicked,
    Dropped,
}

impl Exit {
    fn expected(&self) -> Option<Expect> {
        match self {
            Exit::Ended(ended) => Some(Expect::Ended(*ended)),
            Exit::Failed(InputError::Write(_)) => Some(Expect::WriteError),
            Exit::Failed(_) => None,
            Exit::Panicked => Some(Expect::Panicked),
            Exit::Dropped => Some(Expect::Dropped),
        }
    }
}

/// One way for a run to end.
struct Case {
    name: &'static str,
    opened: Opened,
    settings: ClickerSettings,
    /// Makes the run's signals, given its presses.
    signals: fn(&Presses) -> Signals<Signal, Signal>,
    drop_once_pressed: bool,
    expect: Expect,
}

fn quiet(_: &Presses) -> Signals<Signal, Signal> {
    signals(never(), never())
}

fn cases() -> [Case; 6] {
    let slow = clicks(Duration::from_secs(10), None);
    let failing = open(false);
    failing
        .log
        .fail_once(Call::Button(Button::Left, PressState::Down));
    let case = |name, opened, settings, signals, expect| Case {
        name,
        opened,
        settings,
        signals,
        drop_once_pressed: false,
        expect,
    };
    [
        case(
            "finished",
            open(false),
            clicks(ms(10), Some(5)),
            quiet,
            Expect::Ended(Ended::Finished),
        ),
        case(
            "stopped",
            open(false),
            slow,
            |during| signals(after_the_press(during), never()),
            Expect::Ended(Ended::Stopped),
        ),
        case(
            "shut down",
            open(false),
            slow,
            |during| signals(never(), after_the_press(during)),
            Expect::Ended(Ended::ShutDown),
        ),
        case("write error", failing, slow, quiet, Expect::WriteError),
        case("panicked", open(true), slow, quiet, Expect::Panicked),
        Case {
            drop_once_pressed: true,
            ..case("dropped", open(false), slow, quiet, Expect::Dropped)
        },
    ]
}

#[test]
fn every_way_a_run_ends_leaves_no_thread_and_hands_throttling_back() {
    let _serial = one_at_a_time();
    let display = display_is_ours();
    assert_idle("before the runs", display);
    for case in cases() {
        let name = case.name;
        let signals = (case.signals)(&case.opened.during);
        let ran = run_case(case.opened, &case.settings, signals, case.drop_once_pressed);
        assert_idle(name, display);
        // Opted out, and the display kept on when that can be told, at
        // every press.
        assert!(!ran.during.is_empty(), "{name}: nothing pressed");
        let active = |during: &During| during.opted_out && (during.display_on || !display);
        assert!(ran.during.iter().all(active), "{name}: {:?}", ran.during);
        // Whatever ended it, the last press was released.
        let released = Call::Button(Button::Left, PressState::Up);
        assert_eq!(ran.calls.last(), Some(&released), "{name}");
        // Stop, shutdown and the drop end the 10 s wait at once.
        assert!(
            ran.took < Duration::from_secs(1),
            "{name}: took {:?}",
            ran.took
        );
        assert_eq!(
            ran.exit.expected(),
            Some(case.expect),
            "{name}: {:?}",
            ran.exit
        );
    }
}

#[test]
fn a_stop_while_the_thread_starts_ends_the_run_with_nothing_pressed() {
    let _serial = one_at_a_time();
    // The thread takes 5 s to be ready unless stopped first.
    let _slow = SlowStart::set(5_000);
    let slow = clicks(Duration::from_secs(10), None);
    let ran = run_case(open(false), &slow, signals(after(ms(50)), never()), false);

    let ended = ran.exit.expected();
    assert_eq!(
        (ended, ran.calls),
        (Some(Expect::Ended(Ended::Stopped)), Vec::new())
    );
    assert!(ran.took < Duration::from_secs(1), "took {:?}", ran.took);
}
