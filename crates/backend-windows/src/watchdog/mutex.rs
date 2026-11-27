//! The watchdogs' named mutex, `Local\io.github.tobylo.AutoClicker.Watchdog`
//! (#114): a watchdog whose daemon ended with releases owed holds it until
//! it has sent them all, and a daemon that starts meanwhile refuses starts
//! until it is free, so a new run never presses a key an earlier watchdog
//! is about to send a release for.
//!
//! A watchdog takes it only once its daemon has ended: one held from the
//! watchdog's start would keep its own daemon refused for good. So a new
//! daemon can look between an old daemon's end and its watchdog taking the
//! mutex; that window is narrow, and its worst outcome a stray key-up.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, GetLastError, HANDLE, WAIT_ABANDONED_0, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, INFINITE, MUTEX_MODIFY_STATE, OpenMutexW, ReleaseMutex,
    SYNCHRONIZATION_SYNCHRONIZE, WaitForMultipleObjects, WaitForSingleObject,
};

use crate::instance::error_code;
use crate::message::wide;
use crate::resend::{Also, Stop};

/// The mutex's name in the session's namespace.
const NAME: &str = r"Local\io.github.tobylo.AutoClicker.Watchdog";

/// The mutex as a watchdog holds it, or waits to: released, if held, when
/// dropped, on the thread that took it (a mutex belongs to its thread).
#[derive(Debug)]
pub(crate) struct Holding {
    mutex: OwnedHandle,
    held: bool,
}

impl Holding {
    /// Creates or opens the mutex and takes it if it is free; an earlier
    /// watchdog's is taken once that one lets go ([`Also`]).
    ///
    /// # Errors
    /// Windows could not create or open it.
    pub(crate) fn take() -> io::Result<Holding> {
        Holding::take_named(NAME)
    }

    #[allow(unsafe_code)]
    fn take_named(name: &str) -> io::Result<Holding> {
        let name = wide(name);
        // SAFETY: `name` is NUL-terminated UTF-16, alive for the call;
        // default security, not taken on creation.
        let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a new handle that nothing else owns.
        let mutex = unsafe { OwnedHandle::from_raw_handle(handle) };
        // SAFETY: the mutex is open; no wait.
        let now = unsafe { WaitForSingleObject(mutex.as_raw_handle(), 0) };
        let held = match now {
            WAIT_OBJECT_0 | WAIT_ABANDONED_0 => true,
            WAIT_TIMEOUT => false,
            _ => return Err(io::Error::last_os_error()),
        };
        Ok(Holding { mutex, held })
    }

    /// Whether this watchdog holds it.
    #[cfg(test)]
    pub(crate) fn held(&self) -> bool {
        self.held
    }
}

impl Also for Holding {
    fn handle(&self) -> Option<HANDLE> {
        (!self.held).then(|| self.mutex.as_raw_handle())
    }

    fn taken(&mut self, _abandoned: bool) {
        self.held = true;
    }
}

impl Drop for Holding {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        if self.held {
            // SAFETY: held by this thread, which took it; released once.
            unsafe { ReleaseMutex(self.mutex.as_raw_handle()) };
        }
    }
}

/// An earlier watchdog's mutex, found by a daemon at its start.
#[derive(Debug)]
pub struct Earlier(OwnedHandle);

/// Opens the mutex if an earlier watchdog made it: `None` when none did.
/// One that exists may be free already; [`Earlier::wait`] finds out.
///
/// # Errors
/// Windows could say neither (an object of another kind holds the name,
/// say).
pub fn earlier_watchdog() -> io::Result<Option<Earlier>> {
    open_named(NAME)
}

#[allow(unsafe_code)]
fn open_named(name: &str) -> io::Result<Option<Earlier>> {
    let name = wide(name);
    // SAFETY: `name` is NUL-terminated UTF-16, alive for the call; enough
    // access to wait on the mutex and let go of it.
    let handle = unsafe {
        OpenMutexW(
            SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE,
            0,
            name.as_ptr(),
        )
    };
    if handle.is_null() {
        // SAFETY: reads this thread's last error; no arguments.
        let error = unsafe { GetLastError() };
        if error == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        return Err(io::Error::from_raw_os_error(error_code(error)));
    }
    // SAFETY: a new handle that nothing else owns.
    Ok(Some(Earlier(unsafe {
        OwnedHandle::from_raw_handle(handle)
    })))
}

/// The wait on an earlier watchdog's mutex, until it is free or stopped.
#[derive(Debug)]
pub struct EarlierWait {
    stop: std::sync::Arc<Stop>,
    done: mpsc::Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl Earlier {
    /// Waits on a thread of its own until the mutex is free (abandoned
    /// counts as free), lets go of it at once, and calls `free`. A wait
    /// that fails calls it too, logged by the caller from what it is
    /// given: a daemon must not refuse starts for good. A stopped wait
    /// never calls it.
    ///
    /// # Errors
    /// The stop event or the thread could not be created.
    pub fn wait(
        self,
        free: impl FnOnce(Option<io::Error>) + Send + 'static,
    ) -> io::Result<EarlierWait> {
        let stop = std::sync::Arc::new(Stop::new()?);
        let shared = std::sync::Arc::clone(&stop);
        let (finished, done) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("earlier watchdog".to_owned())
            .spawn(move || {
                match wait_free(&self.0, &shared) {
                    Freed::Stopped => {}
                    Freed::Free => free(None),
                    Freed::Failed(err) => free(Some(err)),
                }
                let _ = finished.send(());
            })?;
        Ok(EarlierWait {
            stop,
            done,
            thread: Some(thread),
        })
    }
}

/// What ended the wait on an earlier watchdog.
enum Freed {
    Stopped,
    Free,
    Failed(io::Error),
}

/// Waits until `mutex` is free or `stop` is set; a mutex taken is let go
/// of at once.
#[allow(unsafe_code)]
fn wait_free(mutex: &OwnedHandle, stop: &Stop) -> Freed {
    let both: [HANDLE; 2] = [stop.raw(), mutex.as_raw_handle()];
    // SAFETY: both handles are open for the call.
    let woken = unsafe { WaitForMultipleObjects(2, both.as_ptr(), 0, INFINITE) };
    match woken {
        WAIT_OBJECT_0 => Freed::Stopped,
        woken if woken == WAIT_OBJECT_0 + 1 || woken == WAIT_ABANDONED_0 + 1 => {
            // SAFETY: taken by this thread just now; released once.
            unsafe { ReleaseMutex(mutex.as_raw_handle()) };
            Freed::Free
        }
        _ => Freed::Failed(io::Error::last_os_error()),
    }
}

impl EarlierWait {
    /// Stops the wait and waits for its thread, at most `within`; whether
    /// it ended in time.
    pub fn stop(mut self, within: Duration) -> bool {
        self.stop.set();
        let ended = !matches!(
            self.done.recv_timeout(within),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        if ended && let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        ended
    }
}

impl Drop for EarlierWait {
    /// Ends the wait without waiting for its thread.
    fn drop(&mut self) {
        self.stop.set();
    }
}

#[cfg(test)]
mod tests;
