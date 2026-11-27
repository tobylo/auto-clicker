//! Where a pass sleeps between wakes: on this thread's messages, a stop
//! event, and whatever else the caller waits for (the watchdog's mutex), in
//! `MsgWaitForMultipleObjectsEx`, with the next timed try's deadline as its
//! timeout and none while no try is due.
//!
//! While something is owed, the thread hooks `EVENT_SYSTEM_FOREGROUND` and
//! `EVENT_SYSTEM_DESKTOPSWITCH` out of context: Windows queues them to this
//! thread, which handles them as it pumps its messages. The hooks go on
//! before the first pass, so a change after that pass's check is heard; they
//! go once nothing is owed. A window of the thread's (the watchdog's)
//! wakes a pass the same way, on an unlock or a reconnect ([`wake_here`]),
//! and ends the wait at the session's end ([`stop_here`]).

use std::cell::Cell;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use std::time::Instant;

use auto_clicker_core::input::InputBackend;
use auto_clicker_core::log;
use windows_sys::Win32::Foundation::{
    HANDLE, HWND, WAIT_ABANDONED_0, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent};
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_DESKTOPSWITCH, EVENT_SYSTEM_FOREGROUND, MSG,
    MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, QS_ALLINPUT,
    TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_QUIT,
};

use super::{Cause, Next, Resend, Via};
use crate::watched::Probe;

thread_local! {
    /// Set by a hook or the thread's window: a pass is due.
    static WOKEN: Cell<bool> = const { Cell::new(false) };
    /// Set by the thread's window at the session's end.
    static STOPPING: Cell<bool> = const { Cell::new(false) };
    /// How many of the thread's hooks are on.
    static HOOKED: Cell<usize> = const { Cell::new(0) };
}

/// A pass is due on this thread: a window of its own heard an unlock or a
/// reconnect.
pub(crate) fn wake_here() {
    WOKEN.set(true);
}

/// The wait on this thread ends: a window of its own heard the session end.
pub(crate) fn stop_here() {
    STOPPING.set(true);
}

/// Whether any of this thread's hooks are on.
#[cfg(test)]
pub(crate) fn hooked_here() -> bool {
    HOOKED.get() > 0
}

/// Whether a pass was woken and the wait stopped on this thread, each
/// taken.
#[cfg(test)]
pub(crate) fn take_signals_here() -> (bool, bool) {
    (WOKEN.take(), STOPPING.take())
}

/// Why the wait ended.
#[derive(Debug)]
pub enum Ended {
    /// Every release was delivered.
    Delivered,
    /// It was stopped, or the session ends; what is left stays owed.
    Stopped,
    /// The wait itself failed; what is left stays owed.
    Failed(io::Error),
}

/// An event that stops a wait at once, from any thread.
#[derive(Debug)]
pub struct Stop(OwnedHandle);

impl Stop {
    /// A stop not yet set.
    ///
    /// # Errors
    /// Windows could not create the event.
    #[allow(unsafe_code)]
    pub fn new() -> io::Result<Stop> {
        // SAFETY: default security, manual reset, not signalled, no name.
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a new handle that nothing else owns.
        Ok(Stop(unsafe { OwnedHandle::from_raw_handle(event) }))
    }

    /// Sets it: every wait on it ends.
    #[allow(unsafe_code)]
    pub fn set(&self) {
        // SAFETY: the event is open for as long as `self`.
        unsafe { SetEvent(self.0.as_raw_handle()) };
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.0.as_raw_handle()
    }
}

/// Something else the wait sleeps on until it is signalled: the watchdog's
/// mutex, held by an earlier watchdog.
pub trait Also {
    /// The handle to wait on, while there is one.
    fn handle(&self) -> Option<HANDLE>;
    /// The wait took it; `abandoned` when its holder ended without letting
    /// go (a mutex).
    fn taken(&mut self, abandoned: bool);
}

/// Sends `resend`'s releases through `via` until every one is delivered or
/// `stop` is set. The hooks are on for as long as it runs. Must run on a
/// thread that may own hooks and windows, and on that thread only.
pub fn send_until_delivered<O: InputBackend, P: Probe>(
    resend: &mut Resend,
    via: &mut Via<O, P>,
    stop: &Stop,
    mut also: Option<&mut dyn Also>,
) -> Ended {
    let _hooks = Hooks::on();
    WOKEN.set(false);
    STOPPING.set(false);
    let mut next = resend.pass(Cause::Event, Instant::now(), via);
    loop {
        let deadline = match next {
            Next::Done => return Ended::Delivered,
            Next::Events => None,
            Next::Until(deadline) => Some(deadline),
        };
        match sleep(stop, deadline, also.as_deref_mut()) {
            Ok(Woke::Stopped) => return Ended::Stopped,
            Ok(Woke::Other) => {}
            Err(err) => return Ended::Failed(err),
        }
        if STOPPING.take() {
            return Ended::Stopped;
        }
        let now = Instant::now();
        if let Some(cause) = cause(deadline, now) {
            next = resend.pass(cause, now, via);
        }
    }
}

/// What a pass is due for after a sleep, at `now`: an event the thread
/// heard, or a timed try's `deadline` passed; none for anything else.
fn cause(deadline: Option<Instant>, now: Instant) -> Option<Cause> {
    if WOKEN.take() {
        return Some(Cause::Event);
    }
    deadline
        .is_some_and(|deadline| deadline <= now)
        .then_some(Cause::Timer)
}

/// What ended one sleep.
enum Woke {
    Stopped,
    /// Messages were handled, `also` was taken, or the deadline passed.
    Other,
}

/// Sleeps until `stop` is set, `also` is signalled, a message arrives
/// (handled before this returns) or `deadline` passes.
#[allow(unsafe_code)]
fn sleep<A: Also + ?Sized>(
    stop: &Stop,
    deadline: Option<Instant>,
    also: Option<&mut A>,
) -> io::Result<Woke> {
    let extra = also.as_ref().and_then(|also| also.handle());
    let mut handles = vec![stop.raw()];
    handles.extend(extra);
    let count = u32::try_from(handles.len()).unwrap_or(u32::MAX);
    let timeout = deadline.map_or(INFINITE, timeout_until);
    // SAFETY: every handle is open for the call (the stop's for as long
    // as `stop`, the other's while `also` offers it); any message of this
    // thread wakes it, input seen before included.
    let woken = unsafe {
        MsgWaitForMultipleObjectsEx(
            count,
            handles.as_ptr(),
            timeout,
            QS_ALLINPUT,
            MWMO_INPUTAVAILABLE,
        )
    };
    woke(woken, count, also)
}

/// What the wait's result `woken` says, for `count` handles and `also`'s:
/// messages are handled here, and `also` is told it was taken.
fn woke<A: Also + ?Sized>(woken: u32, count: u32, also: Option<&mut A>) -> io::Result<Woke> {
    match woken {
        WAIT_OBJECT_0 => Ok(Woke::Stopped),
        WAIT_TIMEOUT => Ok(Woke::Other),
        WAIT_FAILED => Err(io::Error::last_os_error()),
        woken if woken == WAIT_OBJECT_0 + count => {
            Ok(if pump() { Woke::Other } else { Woke::Stopped })
        }
        woken => taken(woken, also),
    }
}

/// Tells `also` it was taken, when `woken` says so.
fn taken<A: Also + ?Sized>(woken: u32, also: Option<&mut A>) -> io::Result<Woke> {
    let abandoned = woken == WAIT_ABANDONED_0 + 1;
    match also {
        Some(also) if abandoned || woken == WAIT_OBJECT_0 + 1 => {
            also.taken(abandoned);
            Ok(Woke::Other)
        }
        _ => Err(io::Error::other(format!(
            "the wait returned {woken:#x}, which it was not given"
        ))),
    }
}

/// The wait's timeout in milliseconds until `deadline`, rounded up so the
/// deadline has passed when it ends; short of `INFINITE`.
fn timeout_until(deadline: Instant) -> u32 {
    let left = deadline.saturating_duration_since(Instant::now());
    let millis = left.as_micros().div_ceil(1000);
    u32::try_from(millis).map_or(INFINITE - 1, |millis| millis.min(INFINITE - 1))
}

/// Handles every message waiting on this thread, the hooks' events among
/// them; `false` on `WM_QUIT`, which ends the wait.
#[allow(unsafe_code)]
fn pump() -> bool {
    let mut message = MSG::default();
    // SAFETY: `message` is a live, writable `MSG`; every message of this
    // thread, removed as it is read.
    while unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } != 0 {
        if message.message == WM_QUIT {
            return false;
        }
        // SAFETY: `message` was just filled in by `PeekMessageW`.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    true
}

/// The thread's hooks on the window in front and the input desktop, until
/// dropped.
struct Hooks(Vec<HWINEVENTHOOK>);

impl Hooks {
    /// Hooks both events; one that cannot be hooked is logged, and only
    /// timed tries, unlocks and reconnects wake a pass for it.
    fn on() -> Hooks {
        let hooks = [EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_DESKTOPSWITCH]
            .into_iter()
            .filter_map(hook)
            .collect::<Vec<_>>();
        HOOKED.set(HOOKED.get().saturating_add(hooks.len()));
        Hooks(hooks)
    }
}

/// Hooks `event` on this thread, out of context.
#[allow(unsafe_code)]
fn hook(event: u32) -> Option<HWINEVENTHOOK> {
    // SAFETY: an out-of-context hook of this thread's, every process and
    // thread, its procedure a plain function; removed when `Hooks` drops.
    let hook = unsafe {
        SetWinEventHook(
            event,
            event,
            null_mut(),
            Some(changed),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.is_null() {
        log::line(format_args!(
            "auto-clicker: window event {event:#x} cannot be heard: {}",
            io::Error::last_os_error()
        ));
        return None;
    }
    Some(hook)
}

impl Drop for Hooks {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        for &hook in &self.0 {
            // SAFETY: a hook `hook` set on this thread, removed once.
            unsafe { UnhookWinEvent(hook) };
        }
        HOOKED.set(HOOKED.get().saturating_sub(self.0.len()));
    }
}

/// A WinEvent hook's procedure that reads none of what Windows passes it
/// (the hook, the event, the window, the object, the child, the thread and
/// the time, in Windows' fixed signature) and runs `$body`.
macro_rules! win_event_procedure {
    ($name:ident => $body:expr) => {
        #[allow(unsafe_code)]
        unsafe extern "system" fn $name(
            _: HWINEVENTHOOK,
            _: u32,
            _: HWND,
            _: i32,
            _: i32,
            _: u32,
            _: u32,
        ) {
            $body
        }
    };
}

// The hooks' procedure: Windows calls it on this thread, as it pumps.
win_event_procedure!(changed => wake_here());

#[cfg(test)]
mod tests;
