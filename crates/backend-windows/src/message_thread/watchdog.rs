//! The daemon's wait on its watchdog's process (M5 brief decision 6): a
//! thread of its own sleeps on the process handle and on a stop event, and
//! when the watchdog ends it posts to the message thread's window, which
//! hands the handler [`WindowEvent::WatchdogEnded`]. Posted, it reaches the
//! window inside the tray menu's own loop too, so an open menu does not
//! hold it up.
//!
//! [`WindowEvent::WatchdogEnded`]: super::WindowEvent::WatchdogEnded

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
use windows_sys::Win32::UI::WindowsAndMessaging::WM_APP;

use super::Remote;

/// Posted when the watchdog's process ended, or can no longer be waited
/// on.
pub(super) const WM_WATCHDOG_ENDED: u32 = WM_APP + 5;

/// The wait on the watchdog, until it is stopped or dropped.
pub struct WatchdogWait {
    stop: Arc<OwnedHandle>,
    done: mpsc::Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl Remote {
    /// Starts waiting on `watchdog`, a process handle with SYNCHRONIZE
    /// access: when the process ends, the handler hears
    /// [`super::WindowEvent::WatchdogEnded`]. A wait that fails says so the
    /// same way, since the daemon must not run unwatched.
    ///
    /// # Errors
    /// The stop event or the thread could not be created.
    #[allow(unsafe_code)]
    pub fn wait_for_watchdog(&self, watchdog: OwnedHandle) -> io::Result<WatchdogWait> {
        // SAFETY: default security, manual reset, not signalled, no name.
        let stop = unsafe { CreateEventW(null(), 1, 0, null()) };
        if stop.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a new handle that nothing else owns.
        let stop = Arc::new(unsafe { OwnedHandle::from_raw_handle(stop) });
        let waited = (Arc::clone(&stop), watchdog);
        let remote = self.clone();
        let (finished, done) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("watchdog wait".to_owned())
            .spawn(move || {
                if wait(&waited.0, &waited.1) {
                    // A window already gone has nobody left to tell.
                    let _ = remote.post(WM_WATCHDOG_ENDED, 0, 0);
                }
                let _ = finished.send(());
            })?;
        Ok(WatchdogWait {
            stop,
            done,
            thread: Some(thread),
        })
    }
}

/// Waits until the watchdog ends (true) or `stop` is set (false). A wait
/// that fails counts as the watchdog's end.
#[allow(unsafe_code)]
fn wait(stop: &OwnedHandle, watchdog: &OwnedHandle) -> bool {
    let both: [HANDLE; 2] = [stop.as_raw_handle(), watchdog.as_raw_handle()];
    // SAFETY: both handles are open for the call, owned by the caller.
    let woken = unsafe { WaitForMultipleObjects(2, both.as_ptr(), 0, INFINITE) };
    match woken {
        WAIT_OBJECT_0 => false,
        woken if woken == WAIT_OBJECT_0 + 1 => true,
        _ => {
            log::line(format_args!(
                "auto-clicker: the watchdog cannot be waited on: {}",
                io::Error::last_os_error()
            ));
            true
        }
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
