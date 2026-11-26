//! A run's timer on Windows (M5 brief decision 5): a high-resolution
//! waitable timer, waited on by a thread that exists only while the run
//! does, which wakes the run's task at each deadline.
//!
//! tokio's own timer wakes on Windows' default tick, about 15.6 ms, which
//! misses most 10 ms periods. A high-resolution waitable timer keeps them to
//! well under a millisecond (X03) without `timeBeginPeriod`, which would
//! raise the tick for the whole system. While a run is active the process
//! also opts out of execution-speed power throttling ([`throttling`]), so
//! Windows does not slow its timer when the program has no visible window,
//! and the thread asks Windows to keep the display on ([`display`]).
//!
//! The run hands the thread each deadline with a number. The thread arms the
//! timer for the time left until it, measured on the monotonic clock when it
//! arms, so a change of the wall clock moves nothing, and when the timer
//! fires it marks that number fired and wakes the task. Only the latest
//! number completes a wait, so a wake meant for a deadline the run has moved
//! on from only polls again. Each run has its own thread and state, so
//! nothing of one run reaches the next.

mod display;
mod throttling;

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use auto_clicker_core::input::{InputError, RunTimer};
use auto_clicker_core::log;
use windows_sys::Win32::Foundation::{HANDLE, WAIT_FAILED, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateEventW, CreateWaitableTimerExW, INFINITE,
    SetEvent, SetWaitableTimer, TIMER_ALL_ACCESS, WaitForMultipleObjects,
};

use display::DisplayOn;
#[cfg(test)]
pub(crate) use display::display_kept_on;
use throttling::Throttling;
#[cfg(test)]
pub(crate) use throttling::execution_speed_managed;

/// How long dropping a timer waits for its thread. The thread only waits on
/// handles, so it ends at once when told to; past this, it is left to end
/// with the process rather than holding up the run's end.
const JOIN_WITHIN: Duration = Duration::from_secs(1);

/// One run's timer. Dropping it ends its thread and, after the last run's
/// timer, the opt-out from power throttling.
pub(crate) struct WaitableTimer {
    shared: Arc<Shared>,
    /// The number the next new deadline gets.
    next: u64,
    thread: Option<JoinHandle<()>>,
    /// Sent on once the thread is ready, and closed when it ends, however
    /// it ends.
    done: mpsc::Receiver<()>,
    _throttling: Throttling,
}

/// What the run's task and the thread share.
struct Shared {
    /// Set to end the thread (manual reset).
    stop: OwnedHandle,
    /// Set when the run asks for a new deadline (auto reset).
    changed: OwnedHandle,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The latest deadline the run asked for.
    request: Option<Request>,
    /// The number of the latest request the timer fired for; 0 for none.
    fired: u64,
    /// The task to wake when the current request fires.
    waker: Option<Waker>,
    /// Why the thread stopped serving, once it has.
    failed: Option<(io::ErrorKind, String)>,
}

#[derive(Clone, Copy, Debug)]
struct Request {
    number: u64,
    deadline: Instant,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Nothing under the lock can panic part way through an update.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl WaitableTimer {
    /// Creates the timer and starts its thread.
    ///
    /// # Errors
    /// When Windows cannot create the timer, its events or its thread.
    pub(crate) fn start() -> io::Result<WaitableTimer> {
        let timer = create_timer()?;
        let shared = Arc::new(Shared {
            stop: create_event(true)?,
            changed: create_event(false)?,
            state: Mutex::default(),
        });
        let (finished, done) = mpsc::channel::<()>();
        let served = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("run timer".to_owned())
            .spawn(move || {
                // Dropped last, on any way out: the joiner hears of the end.
                let finished = finished;
                #[cfg(test)]
                let _live = tests::Live::new();
                let _display = DisplayOn::request();
                // The starter waits for this, so the display is kept on
                // before the run's first press.
                let _ = finished.send(());
                serve_until_stopped(&served, &timer);
            })?;
        let started = WaitableTimer {
            shared,
            next: 1,
            thread: Some(thread),
            done,
            _throttling: Throttling::opt_out(),
        };
        match started.done.recv_timeout(JOIN_WITHIN) {
            Ok(()) => Ok(started),
            // Dropping `started` ends the thread, should it start later.
            Err(_) => Err(io::Error::other("the run timer's thread did not start")),
        }
    }
}

impl RunTimer for WaitableTimer {
    fn poll_until(
        &mut self,
        cx: &mut Context<'_>,
        deadline: tokio::time::Instant,
    ) -> Poll<Result<(), InputError>> {
        let deadline = deadline.into_std();
        let mut state = self.shared.lock();
        if let Some((kind, why)) = &state.failed {
            return Poll::Ready(Err(InputError::Timer(io::Error::new(*kind, why.clone()))));
        }
        if deadline <= Instant::now() {
            return Poll::Ready(Ok(()));
        }
        let current = state.ask(&mut self.next, &self.shared.changed, deadline)?;
        if state.fired == current {
            return Poll::Ready(Ok(()));
        }
        state.wake_later(cx.waker());
        Poll::Pending
    }
}

impl State {
    /// The number of the request for `deadline`: the current one, or a new
    /// one, numbered from `next` and handed to the thread through `changed`.
    fn ask(
        &mut self,
        next: &mut u64,
        changed: &OwnedHandle,
        deadline: Instant,
    ) -> Result<u64, InputError> {
        if let Some(request) = self.request.filter(|request| request.deadline == deadline) {
            return Ok(request.number);
        }
        let number = *next;
        *next = number.saturating_add(1);
        self.request = Some(Request { number, deadline });
        set_event(changed).map_err(InputError::Timer)?;
        Ok(number)
    }

    /// Keeps `waker` to wake when the current request fires.
    fn wake_later(&mut self, waker: &Waker) {
        if !self
            .waker
            .as_ref()
            .is_some_and(|kept| kept.will_wake(waker))
        {
            self.waker = Some(waker.clone());
        }
    }
}

impl Drop for WaitableTimer {
    /// Ends the thread and waits for it, at most [`JOIN_WITHIN`]. The
    /// opt-out is undone after this, as the last field drops.
    fn drop(&mut self) {
        if let Err(err) = set_event(&self.shared.stop) {
            log::line(format_args!(
                "auto-clicker: could not end the run timer's thread: {err}"
            ));
        }
        match self.done.recv_timeout(JOIN_WITHIN) {
            Err(mpsc::RecvTimeoutError::Timeout) => log::line(format_args!(
                "auto-clicker: the run timer's thread did not end within {JOIN_WITHIN:?}; \
                 it is left to end with the program"
            )),
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(thread) = self.thread.take() {
                    // A panic was already caught and reported by the thread.
                    let _ = thread.join();
                }
            }
        }
    }
}

/// The thread's body. A failure or a panic is recorded for the run, whose
/// next poll ends it with the error.
fn serve_until_stopped(shared: &Shared, timer: &OwnedHandle) {
    let failure = match catch_unwind(AssertUnwindSafe(|| serve(shared, timer))) {
        Ok(Ok(())) => return,
        Ok(Err(err)) => (err.kind(), err.to_string()),
        Err(_) => (
            io::ErrorKind::Other,
            "the run timer's thread panicked".to_owned(),
        ),
    };
    let waker = {
        let mut state = shared.lock();
        state.failed = Some(failure);
        state.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// Waits on the stop event, the change event and the timer until stopped.
fn serve(shared: &Shared, timer: &OwnedHandle) -> io::Result<()> {
    let handles = [&shared.stop, &shared.changed, timer].map(AsRawHandle::as_raw_handle);
    let mut armed: Option<Request> = None;
    loop {
        match wait_any(&handles)? {
            0 => return Ok(()),
            1 => {
                armed = shared.lock().request;
                if let Some(request) = armed {
                    arm(timer, request.deadline)?;
                }
            }
            _ => {
                if let Some(request) = armed.take() {
                    fire(shared, request);
                }
            }
        }
    }
}

/// Marks `request` fired and wakes the task, if the run still waits for it.
fn fire(shared: &Shared, request: Request) {
    let waker = {
        let mut state = shared.lock();
        if state.request.map(|current| current.number) != Some(request.number) {
            return;
        }
        state.fired = request.number;
        state.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

#[allow(unsafe_code)]
fn create_timer() -> io::Result<OwnedHandle> {
    // SAFETY: no attributes and no name; the flags and access are valid
    // for a waitable timer.
    let handle = unsafe {
        CreateWaitableTimerExW(
            null(),
            null(),
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
            TIMER_ALL_ACCESS,
        )
    };
    owned(handle)
}

/// An unnamed event, not set.
#[allow(unsafe_code)]
fn create_event(manual_reset: bool) -> io::Result<OwnedHandle> {
    // SAFETY: no attributes and no name.
    let handle = unsafe { CreateEventW(null(), i32::from(manual_reset), 0, null()) };
    owned(handle)
}

#[allow(unsafe_code)]
fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a new handle that nothing else owns; `OwnedHandle` closes it
    // once.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

#[allow(unsafe_code)]
fn set_event(event: &OwnedHandle) -> io::Result<()> {
    // SAFETY: `event` is an open event handle, alive for the call.
    if unsafe { SetEvent(event.as_raw_handle()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Arms `timer` to fire once at `deadline`, from the time left now.
#[allow(unsafe_code)]
fn arm(timer: &OwnedHandle, deadline: Instant) -> io::Result<()> {
    let due = relative_due_time(deadline.saturating_duration_since(Instant::now()));
    // SAFETY: `timer` is an open waitable timer, `due` lives for the call,
    // and there is no completion routine.
    let armed = unsafe { SetWaitableTimer(timer.as_raw_handle(), &due, 0, None, null_mut(), 0) };
    if armed == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `left` as `SetWaitableTimer` takes a relative due time: negative, in
/// 100 ns units, and at least one unit, since 0 would be an absolute time.
fn relative_due_time(left: Duration) -> i64 {
    let units = i64::try_from(left.as_nanos() / 100).unwrap_or(i64::MAX);
    -units.max(1)
}

/// Waits for any of `handles` and returns its index.
#[allow(unsafe_code)]
fn wait_any(handles: &[HANDLE; 3]) -> io::Result<usize> {
    // SAFETY: three open handles, alive for the call; wait for any, with
    // no time-out.
    let result = unsafe { WaitForMultipleObjects(3, handles.as_ptr(), 0, INFINITE) };
    if result == WAIT_FAILED {
        return Err(io::Error::last_os_error());
    }
    match result.checked_sub(WAIT_OBJECT_0) {
        Some(index @ 0..=2) => Ok(index as usize),
        _ => Err(io::Error::other(format!(
            "waiting for the run timer returned {result:#x}"
        ))),
    }
}

#[cfg(test)]
mod tests;
