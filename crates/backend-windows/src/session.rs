//! The user's session as Windows reports it: whether it is locked and
//! whether it is the one at the console (or a remote desktop's) right now.
//!
//! Whether it is connected is also kept for this process, from the session
//! notices its window hears ([`follow`]): a disconnected session (a remote
//! desktop left, a switch to another user) has no input, though
//! `OpenInputDesktop` still opens the desktop it gets back on reconnect, so
//! decision 7's check asks this as well ([`connected`]).

use std::io;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU8, Ordering};

use windows_sys::Win32::System::RemoteDesktop::{
    WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTSActive, WTSFreeMemory, WTSINFOEXW,
    WTSQuerySessionInformationW, WTSSessionInfoEx,
};

/// `WTSINFOEX_LEVEL1_W::SessionFlags`: the session is locked.
const SESSION_LOCKED: i32 = 0;
/// `WTSINFOEX_LEVEL1_W::SessionFlags`: the session is not locked.
const SESSION_UNLOCKED: i32 = 1;

/// The session's state at one moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionState {
    /// Whether the session is locked; `None` when Windows does not say.
    pub locked: Option<bool>,
    /// Whether the session is connected to the console or a remote
    /// desktop, rather than switched away from.
    pub active: bool,
}

/// Reads this process's session state.
///
/// # Errors
/// Windows cannot be asked, or answers with something other than the
/// documented structure.
#[allow(unsafe_code)]
pub fn query() -> io::Result<SessionState> {
    let mut buffer = null_mut();
    let mut bytes = 0u32;
    // SAFETY: the handle and session are Windows' own constants for "this
    // machine, this session"; `buffer` and `bytes` are live and writable,
    // and on success Windows allocates the buffer, freed below.
    let read = unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &mut buffer,
            &mut bytes,
        )
    };
    if read == 0 {
        return Err(io::Error::last_os_error());
    }
    let state = if usize::try_from(bytes).is_ok_and(|bytes| bytes >= size_of::<WTSINFOEXW>()) {
        // SAFETY: Windows wrote at least a whole `WTSINFOEXW` to `buffer`,
        // which it aligned for any structure, and it stays alive until it
        // is freed below. The union's only member is level 1, which
        // `Level` is checked to name before it is read.
        let info = unsafe { &*buffer.cast::<WTSINFOEXW>() };
        (info.Level == 1).then(|| {
            // SAFETY: `Level` is 1, so the union holds `WTSInfoExLevel1`.
            let level1 = unsafe { &info.Data.WTSInfoExLevel1 };
            state(level1.SessionFlags, level1.SessionState == WTSActive)
        })
    } else {
        None
    };
    // SAFETY: `buffer` came from the successful call above and is freed
    // once; nothing refers to it afterwards.
    unsafe { WTSFreeMemory(buffer.cast()) };
    state.ok_or_else(|| io::Error::other("Windows answered with an unknown session structure"))
}

/// Whether this process's session is connected, as last heard: unknown
/// until a read or a notice says, and again after a read that failed.
static CONNECTED: AtomicU8 = AtomicU8::new(UNKNOWN);
const UNKNOWN: u8 = 0;
const YES: u8 = 1;
const NO: u8 = 2;

/// What a window of this process heard about its session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Heard {
    /// The session's state was read.
    Read(SessionState),
    /// The state could not be read, or the notices are not registered for.
    Lost,
    /// The session was connected to the console or a remote desktop, or
    /// unlocked, which only the session at the screen can be.
    Connected,
    /// The session was disconnected.
    Disconnected,
}

/// Keeps what `heard` says of the session's connection.
pub(crate) fn follow(heard: Heard) {
    CONNECTED.store(kept(heard), Ordering::Release);
}

/// What is kept for `heard`.
fn kept(heard: Heard) -> u8 {
    match heard {
        Heard::Read(state) if state.active => YES,
        Heard::Read(_) | Heard::Disconnected => NO,
        Heard::Lost => UNKNOWN,
        Heard::Connected => YES,
    }
}

/// Whether this process's session is connected. Until a notice or a read
/// said, it is read now, and counts as not connected when it cannot be:
/// a session not known to be connected is never taken for one.
pub(crate) fn connected() -> bool {
    connected_by(CONNECTED.load(Ordering::Acquire), query)
}

/// Whether `kept` says connected, reading the state with `read` when it
/// does not know. What is read is not kept: a notice heard meanwhile is
/// newer.
fn connected_by(kept: u8, read: impl FnOnce() -> io::Result<SessionState>) -> bool {
    match kept {
        YES => true,
        NO => false,
        _ => read().is_ok_and(|state| state.active),
    }
}

/// The state from the session's flags and whether it is active.
fn state(flags: i32, active: bool) -> SessionState {
    let locked = match flags {
        SESSION_LOCKED => Some(true),
        SESSION_UNLOCKED => Some(false),
        // WTS_SESSIONSTATE_UNKNOWN, or anything Windows adds later.
        _ => None,
    };
    SessionState { locked, active }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flags_say_locked_unlocked_or_nothing() {
        assert_eq!(state(0, true).locked, Some(true));
        assert_eq!(state(1, true).locked, Some(false));
        assert_eq!(state(-1, true).locked, None);
        assert_eq!(state(7, false).locked, None);
        assert!(!state(1, false).active);
    }

    #[test]
    fn the_connection_follows_reads_and_notices_and_a_lost_one_is_read_again() {
        let read = |active| {
            Heard::Read(SessionState {
                locked: Some(false),
                active,
            })
        };
        let cases = [
            (read(true), YES),
            (read(false), NO),
            (Heard::Connected, YES),
            (Heard::Disconnected, NO),
            (Heard::Lost, UNKNOWN),
        ];
        for (heard, expected) in cases {
            assert_eq!(kept(heard), expected, "{heard:?}");
        }
        let never = || -> io::Result<SessionState> { panic!("read although known") };
        assert!(connected_by(YES, never));
        assert!(!connected_by(NO, never));
        // Not known: read, and a read that fails is not connected.
        let state = |active| {
            move || {
                Ok(SessionState {
                    locked: None,
                    active,
                })
            }
        };
        assert!(connected_by(UNKNOWN, state(true)));
        assert!(!connected_by(UNKNOWN, state(false)));
        assert!(!connected_by(UNKNOWN, || Err(io::Error::other("no"))));
    }

    #[test]
    #[ignore = "reads the logged-in desktop's session: run in the WN01 VM through `vm run`"]
    fn the_logged_in_desktop_is_unlocked_and_active() {
        assert_eq!(
            query().unwrap(),
            SessionState {
                locked: Some(false),
                active: true
            }
        );
    }
}
