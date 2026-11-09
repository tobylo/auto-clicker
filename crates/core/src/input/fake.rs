//! A backend for tests: it records every call and fails the ones it is told to.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::{AbsolutePosition, Button, Input, InputBackend, InputError, Key, PressState};

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
}
