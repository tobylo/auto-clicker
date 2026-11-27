//! The watchdog after its daemon ended (WN06b): it sends what the table
//! owed at once, and if any of it may not have arrived, it stays until
//! every release is delivered or the session ends.
//!
//! While it stays it holds the watchdogs' mutex ([`super::mutex`]), so a
//! daemon started meanwhile refuses starts; it hooks the window in front and
//! the input desktop ([`crate::resend`]); and a hidden top-level window of
//! its own hears unlocks and reconnects, which wake a pass, and the
//! session's end, which ends the wait. All of it is set up before the pass
//! that follows, so nothing between the first try and the hooks is missed.

use std::cell::Cell;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use auto_clicker_core::input::{Held, InputBackend};
use auto_clicker_core::log;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, WM_ENDSESSION, WM_QUERYENDSESSION, WM_WTSSESSION_CHANGE,
};

use super::Owed;
use super::mutex::Holding;
use crate::message::wide;
use crate::message_thread::{SessionChange, hidden_window, unregister_class};
use crate::output::Output;
use crate::resend::{self, Also, Cause, Ended, Resend, Stop, Via};
use crate::watched::{Probe, ReadDesktop};

/// What the table owed, once sent the first time.
#[derive(Debug)]
pub struct Released {
    resend: Resend,
    /// How many releases were owed.
    pub owed: usize,
}

impl Released {
    /// How many of them may not have arrived yet.
    pub fn missed(&self) -> usize {
        self.resend.owed()
    }
}

/// Sends each of `owed`'s releases once, at once, as decision 7 judges
/// them: what may not have arrived stays in what this returns.
pub fn release(owed: &Owed) -> Released {
    release_through(owed.items(), &mut desktop())
}

/// SendInput's output, judged on the desktop as it is.
fn desktop() -> Via<Output, ReadDesktop> {
    Via {
        output: Output::bare(),
        probe: ReadDesktop,
    }
}

/// [`release`], through `via`.
pub(super) fn release_through<O: InputBackend, P: Probe>(
    items: &[Held],
    via: &mut Via<O, P>,
) -> Released {
    let mut resend = Resend::new(items.iter().copied());
    resend.pass(Cause::Event, Instant::now(), via);
    Released {
        resend,
        owed: items.len(),
    }
}

/// How the watchdog's stay ended.
#[derive(Debug)]
pub enum Stayed {
    /// Every release was delivered.
    Delivered,
    /// The session ends; this many releases are still owed.
    SessionEnds(usize),
    /// It could not stay (the wait could not be made, or failed); this
    /// many releases are still owed.
    Failed(io::Error, usize),
}

impl Released {
    /// Stays until every release still owed is delivered, or the session
    /// ends. Must run on the thread that sent them, which then owns the
    /// window and the hooks.
    pub fn send_until_delivered(mut self) -> Stayed {
        if self.missed() == 0 {
            return Stayed::Delivered;
        }
        let left = |resend: &Resend| resend.owed();
        let stop = match Stop::new() {
            Ok(stop) => stop,
            Err(err) => return Stayed::Failed(err, left(&self.resend)),
        };
        // Neither is needed to send: without them a new start is not
        // refused meanwhile, or an unlock, a reconnect and the session's
        // end are not heard (the hooks still hear the desktop come back).
        let mut mutex = Holding::take()
            .inspect_err(|err| {
                log::line(format_args!(
                    "auto-clicker-watchdog: the watchdogs' mutex cannot be held ({err}); \
                     a new start is not refused meanwhile"
                ));
            })
            .ok();
        let window = Window::open()
            .inspect_err(|err| {
                log::line(format_args!(
                    "auto-clicker-watchdog: session notices cannot be heard ({err})"
                ));
            })
            .ok();
        let ended = resend::send_until_delivered(
            &mut self.resend,
            &mut desktop(),
            &stop,
            mutex.as_mut().map(|mutex| mutex as &mut dyn Also),
        );
        drop(window);
        drop(mutex);
        match ended {
            Ended::Delivered => Stayed::Delivered,
            Ended::Stopped => Stayed::SessionEnds(left(&self.resend)),
            Ended::Failed(err) => Stayed::Failed(err, left(&self.resend)),
        }
    }
}

/// Names each window's class apart.
static CLASSES: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Whether this thread's window is registered for session notices.
    static WATCHED: Cell<bool> = const { Cell::new(false) };
}

/// The watchdog's hidden top-level window, registered for session notices,
/// until dropped.
struct Window {
    hwnd: HWND,
    class: Vec<u16>,
}

impl Window {
    /// Creates it and registers for session notices. Decision 7's check
    /// reads the session's connection itself ([`ReadDesktop`]): a notice
    /// waits in the queue behind the pass that checks.
    #[allow(unsafe_code)]
    fn open() -> io::Result<Window> {
        let class = wide(&format!(
            "auto-clicker.watchdog-window.{}.{}",
            std::process::id(),
            CLASSES.fetch_add(1, Ordering::Relaxed)
        ));
        let hwnd = hidden_window(&class, window_procedure)?;
        let window = Window { hwnd, class };
        // SAFETY: `hwnd` is this thread's live window.
        if unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } == 0 {
            return Err(io::Error::last_os_error());
        }
        WATCHED.set(true);
        Ok(window)
    }
}

impl Drop for Window {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        if WATCHED.replace(false) {
            // SAFETY: registered above for this live window; a failure
            // leaves a registration that ends with the window.
            unsafe { WTSUnRegisterSessionNotification(self.hwnd) };
        }
        // SAFETY: this thread's window, destroyed once, on its thread.
        unsafe { DestroyWindow(self.hwnd) };
        unregister_class(&self.class);
    }
}

/// The window procedure: an unlock or a reconnect wakes a pass; the session's end ends the wait. Nothing in it
/// can panic.
#[allow(unsafe_code)]
unsafe extern "system" fn window_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match answer(message, wparam) {
        Some(result) => result,
        // SAFETY: the arguments are the ones Windows passed in.
        None => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// What the window answers to `message`; `None` leaves it to Windows.
fn answer(message: u32, wparam: WPARAM) -> Option<LRESULT> {
    match message {
        WM_WTSSESSION_CHANGE => {
            if matches!(
                SessionChange::from_code(wparam),
                Some(SessionChange::Unlock | SessionChange::Connect)
            ) {
                resend::wake_here();
            }
            Some(0)
        }
        // Never holds up a logoff.
        WM_QUERYENDSESSION => Some(1),
        WM_ENDSESSION => {
            if wparam != 0 {
                resend::stop_here();
            }
            Some(0)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
