//! Helpers shared by this crate's tests.

use std::fs;
use std::future::{Future, pending};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::{Instant, sleep};

use crate::input::fake::{Call, FakeBackend, FakeLog};
use crate::input::{AbsolutePosition, Button, Input, InputBackend, InputError, Key, PressState};
use crate::run::Signals;

/// A fresh empty directory under the system temp directory, removed on drop.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new() -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "auto-clicker-test-{}-{:016x}",
            std::process::id(),
            fastrand::u64(..)
        ));
        fs::create_dir(&dir).unwrap();
        TempDir(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    /// The names in the directory, sorted.
    pub(crate) fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Whether file permissions bind this process. They do not for root (CI
/// containers often run as root), so tests that rely on a denied open skip
/// there. Probed by opening a file nobody may read.
pub(crate) fn permissions_apply() -> bool {
    let dir = TempDir::new();
    let probe = dir.path().join("probe");
    fs::write(&probe, b"").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o000)).unwrap();
    fs::File::open(&probe).is_err()
}

/// A call and when it was made, relative to the run's start.
pub(crate) type Stamped = (Duration, Call);

pub(crate) type CallLog = Arc<Mutex<Vec<Stamped>>>;

/// The fake backend, with every call stamped with virtual time.
pub(crate) struct Timed {
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

/// A timed fake input whose clock starts now, its stamped calls, and the
/// fake's own log for injecting failures.
pub(crate) fn timed_input() -> (Input<Timed>, CallLog, FakeLog) {
    let (inner, log) = FakeBackend::new();
    let calls = CallLog::default();
    let backend = Timed {
        inner,
        start: Instant::now(),
        calls: Arc::clone(&calls),
    };
    (Input::new(backend), calls, log)
}

/// Far longer than any run in the tests, in virtual time.
pub(crate) const RUNAWAY: Duration = Duration::from_secs(60 * 60);

pub(crate) type AnySignals =
    Signals<Pin<Box<dyn Future<Output = ()>>>, Pin<Box<dyn Future<Output = ()>>>>;

/// Neither signal ever fires.
pub(crate) fn never() -> AnySignals {
    signals(pending(), pending())
}

pub(crate) fn signals(
    stop: impl Future<Output = ()> + 'static,
    shutdown: impl Future<Output = ()> + 'static,
) -> AnySignals {
    Signals {
        stop: Box::pin(stop),
        shutdown: Box::pin(shutdown),
    }
}

/// Stop fires `after` the start.
pub(crate) fn stop_after(after: Duration) -> AnySignals {
    signals(sleep(after), pending())
}

/// Shutdown fires `after` the start.
pub(crate) fn shutdown_after(after: Duration) -> AnySignals {
    signals(pending(), sleep(after))
}
