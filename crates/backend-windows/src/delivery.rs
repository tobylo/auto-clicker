//! Whether a release reached the window in front (M5 brief decision 7).
//!
//! Windows drops injected input in silence while a window of a higher
//! integrity level (an elevated program) is in front, and refuses it on the
//! secure desktop (the lock screen, a UAC prompt); afterwards it reports the
//! key up although the program in front never saw it come up. So neither
//! `SendInput`'s result nor the key state says whether a release arrived;
//! the context does: a release counts as delivered only when, both before
//! and after it was sent, the user's desktop has input and the window in
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

/// What was in front at one moment, as far as a release is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
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
        self.desktop_has_input && self.foreground != 0 && self.foreground_allowed
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

/// What is in front now.
pub fn context() -> Context {
    let desktop_has_input = input_desktop();
    // SAFETY: no arguments.
    #[allow(unsafe_code)]
    let window = unsafe { GetForegroundWindow() };
    let foreground = window as usize;
    let foreground_allowed = !window.is_null()
        && match (window_integrity(window), own_integrity()) {
            (Some(theirs), Some(ours)) => theirs <= ours,
            _ => false,
        };
    Context {
        desktop_has_input,
        foreground,
        foreground_allowed,
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
        assert_eq!(judge(OPEN, OPEN, &SENT), Delivery::Delivered);
        for blocked in [locked, elevated, nothing] {
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

    #[test]
    fn this_programs_own_integrity_can_be_read() {
        assert!(own_integrity().is_some());
    }
}
