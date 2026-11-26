//! The daemon's wait on its watchdog's process (M5 brief decision 6): a
//! thread of its own sleeps on the process handle and on a stop event, and
//! when the watchdog ends it calls what it was given, which tells the
//! controller directly. Nothing stands between the two that could fail or
//! fill up: not the message thread's queue, nor an open tray menu.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null;
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use auto_clicker_core::log;
use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects,
};

/// The wait on the watchdog, until it is stopped or dropped.
pub struct WatchdogWait {
    stop: Arc<OwnedHandle>,
    done: mpsc::Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

/// Starts waiting on `watchdog`, a process handle with SYNCHRONIZE access:
/// when the process ends, `ended` is called, once, on the wait's thread. A
/// wait that fails calls it too, since the daemon must not run unwatched.
/// A stopped wait never calls it.
///
/// # Errors
/// The stop event or the thread could not be created.
#[allow(unsafe_code)]
pub fn wait_for_watchdog(
    watchdog: OwnedHandle,
    ended: impl FnOnce() + Send + 'static,
) -> io::Result<WatchdogWait> {
    // SAFETY: default security, manual reset, not signalled, no name.
    let stop = unsafe { CreateEventW(null(), 1, 0, null()) };
    if stop.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a new handle that nothing else owns.
    let stop = Arc::new(unsafe { OwnedHandle::from_raw_handle(stop) });
    let waited = (Arc::clone(&stop), watchdog);
    let (finished, done) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("watchdog wait".to_owned())
        .spawn(move || {
            match wait(&waited.0, &waited.1) {
                Woken::Stopped => {}
                Woken::Ended => ended(),
                // Told before the log is written: a log write that stalls
                // must not keep the controller unwatched.
                Woken::Failed(err) => {
                    ended();
                    log::line(format_args!(
                        "auto-clicker: the watchdog cannot be waited on: {err}"
                    ));
                }
            }
            let _ = finished.send(());
        })?;
    Ok(WatchdogWait {
        stop,
        done,
        thread: Some(thread),
    })
}

/// What ended a wait.
enum Woken {
    /// The stop event was set.
    Stopped,
    /// The watchdog's process ended.
    Ended,
    /// The wait failed, which counts as the watchdog's end.
    Failed(io::Error),
}

/// Waits until the watchdog ends or `stop` is set.
#[allow(unsafe_code)]
fn wait(stop: &OwnedHandle, watchdog: &OwnedHandle) -> Woken {
    let both: [HANDLE; 2] = [stop.as_raw_handle(), watchdog.as_raw_handle()];
    // SAFETY: both handles are open for the call, owned by the caller.
    let woken = unsafe { WaitForMultipleObjects(2, both.as_ptr(), 0, INFINITE) };
    match woken {
        WAIT_OBJECT_0 => Woken::Stopped,
        woken if woken == WAIT_OBJECT_0 + 1 => Woken::Ended,
        _ => Woken::Failed(io::Error::last_os_error()),
    }
}

impl WatchdogWait {
    /// Stops the wait and waits for its thread, at most `within`; whether it
    /// ended in time. One that did not goes on sleeping on handles it
    /// shares, and ends with the process.
    #[allow(unsafe_code)]
    pub fn stop(mut self, within: Duration) -> bool {
        // SAFETY: the event is open, shared with the thread.
        unsafe { SetEvent(self.stop.as_raw_handle()) };
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

impl Drop for WatchdogWait {
    /// Ends the wait without waiting for its thread.
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the event is open, shared with the thread.
        unsafe { SetEvent(self.stop.as_raw_handle()) };
    }
}

#[cfg(test)]
mod tests;
