//! Whether a release reached the window in front (M5 brief decision 7).
//!
//! Windows drops injected input in silence while a window of a higher
//! integrity level (an elevated program) is in front, and refuses it on the
//! secure desktop (the lock screen, a UAC prompt); afterwards it reports the
//! key up although the program in front never saw it come up. So neither
//! `SendInput`'s result nor the key state says whether a release arrived;
//! the context does: a release counts as delivered only when, both before
//! and after it was sent, the session is connected, the user's desktop has
//! input and the window in
//! front belongs to a process whose integrity can be read and is not above
//! this program's own, with the same window in front both times
//! (docs/decisions/0006, point 6).

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null_mut;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel,
};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, DESKTOP_READOBJECTS, OpenInputDesktop,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::session;

/// What was in front at one moment, as far as a release is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    /// Whether the session is connected to the console or a remote
    /// desktop: a disconnected one has no input, though its desktop still
    /// opens.
    pub connected: bool,
    /// Whether the user's desktop has input (`OpenInputDesktop` succeeds);
    /// not on the lock screen or a UAC prompt.
    pub desktop_has_input: bool,
    /// The window in front, by its handle's value; 0 for none.
    pub foreground: usize,
    /// Whether that window's process has an integrity this program could
    /// read and that is not above its own.
    pub foreground_allowed: bool,
}

impl Context {
    /// Whether a release sent now could arrive.
    pub fn allows_delivery(self) -> bool {
        self.connected && self.desktop_has_input && self.foreground != 0 && self.foreground_allowed
    }
}

/// What became of a release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// It arrived.
    Delivered,
    /// The context did not allow it, before or after, or changed in
    /// between: it may not have arrived.
    Blocked,
    /// The context allowed it, yet Windows took none of it (another
    /// program blocks input, say).
    Refused,
}

/// What became of a release that was `sent` between `before` and `after`.
pub fn judge(before: Context, after: Context, sent: &Result<(), impl Sized>) -> Delivery {
    if !allowed_throughout(before, after) {
        return Delivery::Blocked;
    }
    if sent.is_err() {
        return Delivery::Refused;
    }
    Delivery::Delivered
}

/// Whether both moments allowed delivery, with the same window in front.
fn allowed_throughout(before: Context, after: Context) -> bool {
    let both = before.allows_delivery() && after.allows_delivery();
    both && before.foreground == after.foreground
}

/// Whether what the program sends now could reach the window in front: a
/// hotkey start is refused when not (WN08).
pub fn foreground_takes_input() -> bool {
    context().allows_delivery()
}

/// What is in front now.
pub fn context() -> Context {
    let connected = session::connected();
    let desktop_has_input = input_desktop();
    // SAFETY: no arguments.
    #[allow(unsafe_code)]
    let window = unsafe { GetForegroundWindow() };
    let foreground = window as usize;
    let foreground_allowed =
        !window.is_null() && not_above(window_integrity(window), own_integrity());
    Context {
        connected,
        desktop_has_input,
        foreground,
        foreground_allowed,
    }
}

/// Whether `theirs`, an integrity level, is known and not above `ours`,
/// itself known: one that cannot be read may be above it.
fn not_above(theirs: Option<u32>, ours: Option<u32>) -> bool {
    match (theirs, ours) {
        (Some(theirs), Some(ours)) => theirs <= ours,
        _ => false,
    }
}

/// Whether the desktop that takes input can be opened: not while the
/// secure desktop (the lock screen, a UAC prompt) has it.
#[allow(unsafe_code)]
fn input_desktop() -> bool {
    // SAFETY: plain call; the handle, if any, is closed below.
    let desktop = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
    if desktop.is_null() {
        return false;
    }
    // SAFETY: opened above and closed once.
    unsafe { CloseDesktop(desktop) };
    true
}

/// The integrity level (the mandatory label's RID) of the process that owns
/// `window`; `None` when it cannot be read, as for a process of a higher
/// level that does not let this one look.
#[allow(unsafe_code)]
fn window_integrity(window: windows_sys::Win32::Foundation::HWND) -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: `window` is any window handle; a stale one gives 0.
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    if pid == 0 {
        return None;
    }
    // SAFETY: plain call; the handle, if any, is owned below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    // SAFETY: a new handle that nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    integrity(process.as_raw_handle()).ok()
}

/// This program's own integrity level, read once.
fn own_integrity() -> Option<u32> {
    static OWN: OnceLock<Option<u32>> = OnceLock::new();
    #[allow(unsafe_code)]
    // SAFETY: the pseudo-handle for this process needs no closing.
    *OWN.get_or_init(|| integrity(unsafe { GetCurrentProcess() }).ok())
}

/// The integrity level of `process`, a handle with limited query access.
#[allow(unsafe_code)]
fn integrity(process: HANDLE) -> io::Result<u32> {
    let mut token: HANDLE = null_mut();
    // SAFETY: `process` is a live process handle; `token` is writable.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a new handle that nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    // A label is a SID of one sub-authority after its header; 64 bytes,
    // as `usize`s for alignment, hold it with room to spare.
    let mut buffer = [0usize; 8];
    let mut needed = 0u32;
    let size = u32::try_from(size_of_val(&buffer)).unwrap_or(u32::MAX);
    // SAFETY: `buffer` is `size` writable bytes, aligned for the label.
    let read = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenIntegrityLevel,
            buffer.as_mut_ptr().cast(),
            size,
            &mut needed,
        )
    };
    if read == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success Windows wrote a `TOKEN_MANDATORY_LABEL` at the
    // start of `buffer`, whose SID points inside it; the SID is valid, so
    // its count and last sub-authority can be read.
    unsafe {
        let label = &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>();
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return Err(io::Error::other("the integrity label has no level"));
        }
        Ok(*GetSidSubAuthority(sid, u32::from(count - 1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN: Context = Context {
        connected: true,
        desktop_has_input: true,
        foreground: 7,
        foreground_allowed: true,
    };
    const SENT: Result<(), ()> = Ok(());

    #[test]
    fn a_release_is_delivered_only_with_input_and_the_same_allowed_window_both_times() {
        let locked = Context {
            desktop_has_input: false,
            ..OPEN
        };
        let elevated = Context {
            foreground_allowed: false,
            ..OPEN
        };
        let nothing = Context {
            foreground: 0,
            ..OPEN
        };
        let other = Context {
            foreground: 8,
            ..OPEN
        };
        // A remote desktop left: its desktop opens, but nothing takes input.
        let disconnected = Context {
            connected: false,
            ..OPEN
        };
        assert_eq!(judge(OPEN, OPEN, &SENT), Delivery::Delivered);
        for blocked in [locked, elevated, nothing, disconnected] {
            assert_eq!(
                judge(blocked, OPEN, &SENT),
                Delivery::Blocked,
                "{blocked:?}"
            );
            assert_eq!(
                judge(OPEN, blocked, &SENT),
                Delivery::Blocked,
                "{blocked:?}"
            );
        }
        assert_eq!(judge(OPEN, other, &SENT), Delivery::Blocked);
    }

    #[test]
    fn what_windows_refused_in_an_open_context_is_refused_and_blocked_wins() {
        let refused: Result<(), ()> = Err(());
        assert_eq!(judge(OPEN, OPEN, &refused), Delivery::Refused);
        let locked = Context {
            desktop_has_input: false,
            ..OPEN
        };
        assert_eq!(judge(locked, locked, &refused), Delivery::Blocked);
    }

    /// Windows' integrity levels: low, medium, high and system.
    const LOW: u32 = 0x1000;
    const MEDIUM: u32 = 0x2000;
    const HIGH: u32 = 0x3000;
    const SYSTEM: u32 = 0x4000;

    #[test]
    fn a_window_counts_only_when_its_integrity_is_known_and_not_above_ours() {
        assert!(not_above(Some(MEDIUM), Some(MEDIUM)));
        assert!(not_above(Some(LOW), Some(MEDIUM)));
        // Elevated Task Manager in front of an unelevated program.
        assert!(!not_above(Some(HIGH), Some(MEDIUM)));
        // A process that does not let this one look, or not knowing our own.
        assert!(!not_above(None, Some(MEDIUM)));
        assert!(!not_above(Some(LOW), None));
    }

    #[test]
    fn this_programs_own_integrity_is_one_of_windows_levels() {
        // Unelevated in a desktop, high over the test VM's SSH.
        let own = own_integrity().unwrap();
        assert!([LOW, MEDIUM, HIGH, SYSTEM].contains(&own), "{own:#x}");
        // Read through a handle as for another process's window, the same.
        let pid = std::process::id();
        // SAFETY: plain call; the handle is owned below.
        #[allow(unsafe_code)]
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        assert!(!process.is_null());
        // SAFETY: a new handle that nothing else owns.
        #[allow(unsafe_code)]
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        assert_eq!(integrity(process.as_raw_handle()).unwrap(), own);
    }
}
