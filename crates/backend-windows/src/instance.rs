//! One daemon per session: the first one holds a named mutex in the
//! session's own namespace (`Local\`), and a second one finds it there.
//!
//! The mutex is never acquired; holding a handle to it keeps the name. The
//! kernel closes that handle when the process ends however it ends, so a
//! crash leaves no stale claim.

use std::io;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::ptr::null;

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows_sys::Win32::System::Threading::CreateMutexW;

use crate::message::wide;

/// The claimed name, held until this is dropped.
#[derive(Debug)]
pub struct Instance {
    _mutex: OwnedHandle,
}

/// Why the name could not be claimed.
#[derive(Debug)]
pub enum ClaimError {
    /// Another process in this session holds it.
    Taken,
    /// Windows could not say: the name could not be created or opened (an
    /// object of another kind holds it, say). Never read as "taken" or as
    /// "free".
    Failed(io::Error),
}

/// Claims `name` in this session's namespace. A process that finds the
/// name taken lets go of it before returning, so nothing it does afterwards
/// (showing a message, say) keeps the name alive once its holder is gone.
///
/// # Errors
/// [`ClaimError::Taken`] when another process holds the name, and
/// [`ClaimError::Failed`] when Windows cannot create or open it.
#[allow(unsafe_code)]
pub fn claim(name: &str) -> Result<Instance, ClaimError> {
    let name = wide(&format!("Local\\{name}"));
    // SAFETY: `name` is NUL-terminated UTF-16, alive for the call; no
    // security attributes are given, so the mutex gets the default ones,
    // and it is not acquired.
    let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
    // Read at once: nothing may run between the call and its error.
    // SAFETY: reads this thread's last error; no arguments.
    let error = unsafe { GetLastError() };
    if handle.is_null() {
        return Err(ClaimError::Failed(io::Error::from_raw_os_error(
            error_code(error),
        )));
    }
    // SAFETY: on success `handle` is a new handle to the mutex that nothing
    // else owns; `OwnedHandle` closes it once.
    let mutex = unsafe { OwnedHandle::from_raw_handle(handle) };
    if error == ERROR_ALREADY_EXISTS {
        // Dropping the handle lets go of the name.
        drop(mutex);
        return Err(ClaimError::Taken);
    }
    Ok(Instance { _mutex: mutex })
}

/// A Win32 error code as `io::Error` takes it.
pub(crate) fn error_code(error: u32) -> i32 {
    // Win32 error codes fit in 16 bits; the cast keeps any other bit
    // pattern as Windows' own HRESULT-style value.
    error.cast_signed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(what: &str) -> String {
        format!(
            "io.github.tobylo.AutoClicker.test.{what}.{}.{:?}",
            std::process::id(),
            std::time::Instant::now()
        )
    }

    #[test]
    fn a_second_claim_is_taken_until_the_first_is_dropped() {
        let name = unique("second");
        let first = claim(&name).unwrap();
        assert!(matches!(claim(&name), Err(ClaimError::Taken)));
        // The refused claim let go of its handle: once the first is gone,
        // the name is free again.
        drop(first);
        let again = claim(&name).unwrap();
        drop(again);
    }

    #[test]
    #[allow(unsafe_code)]
    fn a_name_held_by_another_kind_of_object_fails_rather_than_taken() {
        use windows_sys::Win32::System::Threading::CreateEventW;

        let name = unique("event");
        let wide_name = wide(&format!("Local\\{name}"));
        // SAFETY: `wide_name` is NUL-terminated UTF-16, alive for the call;
        // default security, a manual-reset event, not signalled.
        let event = unsafe { CreateEventW(null(), 1, 0, wide_name.as_ptr()) };
        assert!(!event.is_null(), "{}", io::Error::last_os_error());
        // SAFETY: a new handle that nothing else owns.
        let event = unsafe { OwnedHandle::from_raw_handle(event) };
        assert!(matches!(claim(&name), Err(ClaimError::Failed(_))));
        drop(event);
    }
}
