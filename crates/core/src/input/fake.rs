//! A backend for tests: it records every call and fails the ones it is told to.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use tokio::time::Instant;

use super::{
    AbsolutePosition, Button, Input, InputBackend, InputError, Key, PressState, RunTimer,
    TokioTimer,
};

/// One call made to the fake backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    Button(Button, PressState),
    Key(Key, PressState),
    MoveRelative(i32, i32),
    MoveAbsolute(AbsolutePosition),
    Scroll(i32),
}

#[derive(Debug, Default)]
struct State {
    /// Every call that was attempted, failed ones included.
    calls: Vec<Call>,
    /// Calls that fail; each entry fails one matching attempt.
    failures: Vec<Call>,
    /// Run timers made so far, failed attempts included.
    timers: u32,
    /// How many of the next attempts to make a run timer fail.
    timer_failures: u32,
    /// What new run timers do for a deadline ahead: wait for it (`None`),
    /// never fire, or fail.
    ahead: Option<Ahead>,
    /// For each run timer dropped, how many calls had been made by then.
    timer_drops: Vec<usize>,
}

/// The test's view of a [`FakeBackend`], valid after the `Input` that owns
/// the backend is gone (dropped, or unwound by a panic).
#[derive(Clone, Debug, Default)]
pub struct FakeLog(Arc<Mutex<State>>);

impl FakeLog {
    /// Every call attempted so far, in order, failed ones included.
    pub fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    /// Makes the next attempt of `call` fail with a write error. Calling this
    /// twice makes two attempts fail.
    pub fn fail_once(&self, call: Call) {
        self.lock().failures.push(call);
    }

    /// Makes the next attempt to make a run timer fail.
    pub fn fail_timer_once(&self) {
        let mut state = self.lock();
        state.timer_failures = state.timer_failures.saturating_add(1);
    }

    /// How many run timers were asked for, failed attempts included.
    pub fn timers(&self) -> u32 {
        self.lock().timers
    }

    /// Makes later run timers fire only for a deadline that has already
    /// passed when it is first polled, never for one ahead: a run on them
    /// does what is due at its start and then waits until a signal.
    pub fn freeze_timers(&self) {
        self.lock().ahead = Some(Ahead::Never);
    }

    /// Makes later run timers fail for a deadline ahead, as a timer that
    /// stopped working does; one already past is still ready.
    pub fn break_timers(&self) {
        self.lock().ahead = Some(Ahead::Fail);
    }

    /// For each run timer dropped so far, how many calls had been made when
    /// it was dropped.
    pub fn timer_drops(&self) -> Vec<usize> {
        self.lock().timer_drops.clone()
    }

    fn timer(&self) -> Result<Box<dyn RunTimer>, InputError> {
        let mut state = self.lock();
        state.timers = state.timers.saturating_add(1);
        if state.timer_failures > 0 {
            state.timer_failures -= 1;
            return Err(InputError::Timer(std::io::Error::other("injected failure")));
        }
        let inner = match state.ahead {
            Some(ahead) => Box::new(Stuck(ahead)),
            None => TokioTimer::boxed(),
        };
        Ok(Box::new(NotedTimer {
            inner,
            log: self.clone(),
        }))
    }

    fn record(&self, call: Call) -> Result<(), InputError> {
        let mut state = self.lock();
        state.calls.push(call);
        match state.failures.iter().position(|failing| *failing == call) {
            Some(index) => {
                state.failures.remove(index);
                Err(InputError::Write(std::io::Error::other(format!(
                    "injected failure: {call:?}"
                ))))
            }
            None => Ok(()),
        }
    }

    /// A test that panicked while holding the lock leaves the log readable.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A run timer that notes in the log when it is dropped.
struct NotedTimer {
    inner: Box<dyn RunTimer>,
    log: FakeLog,
}

impl RunTimer for NotedTimer {
    fn poll_until(
        &mut self,
        cx: &mut Context<'_>,
        deadline: Instant,
    ) -> Poll<Result<(), InputError>> {
        self.inner.poll_until(cx, deadline)
    }
}

impl Drop for NotedTimer {
    fn drop(&mut self) {
        let mut state = self.log.lock();
        let calls = state.calls.len();
        state.timer_drops.push(calls);
    }
}

/// What a stuck run timer does for a deadline ahead.
#[derive(Clone, Copy, Debug)]
enum Ahead {
    Never,
    Fail,
}

/// A run timer that is ready only for a deadline already past.
struct Stuck(Ahead);

impl RunTimer for Stuck {
    fn poll_until(
        &mut self,
        _cx: &mut Context<'_>,
        deadline: Instant,
    ) -> Poll<Result<(), InputError>> {
        if deadline <= Instant::now() {
            return Poll::Ready(Ok(()));
        }
        match self.0 {
            Ahead::Never => Poll::Pending,
            Ahead::Fail => Poll::Ready(Err(InputError::Timer(std::io::Error::other(
                "injected timer failure",
            )))),
        }
    }
}

/// An input backend that records calls instead of touching a device.
#[derive(Debug)]
pub struct FakeBackend {
    log: FakeLog,
}

impl FakeBackend {
    /// An `Input` over a new fake backend, and the log to inspect it with.
    pub fn open() -> (Input<FakeBackend>, FakeLog) {
        let (backend, log) = FakeBackend::new();
        (Input::new(backend), log)
    }

    /// A bare fake backend, for a test that wraps it itself (in a `Box`, say).
    pub fn new() -> (FakeBackend, FakeLog) {
        let log = FakeLog::default();
        (FakeBackend { log: log.clone() }, log)
    }
}

impl InputBackend for FakeBackend {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        self.log.record(Call::Button(button, state))
    }

    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError> {
        self.log.record(Call::Key(key, state))
    }

    fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        self.log.record(Call::MoveRelative(dx, dy))
    }

    fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError> {
        self.log.record(Call::MoveAbsolute(position))
    }

    fn scroll(&mut self, notches: i32) -> Result<(), InputError> {
        self.log.record(Call::Scroll(notches))
    }

    fn run_timer(&mut self) -> Result<Box<dyn RunTimer>, InputError> {
        self.log.timer()
    }
}
