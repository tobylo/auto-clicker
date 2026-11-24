//! Whether the process runs elevated: as administrator, or as an
//! administrator with User Account Control turned off.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null_mut;

use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// The size Windows is told the answer's buffer has.
const ELEVATION_SIZE: u32 = size_of::<TOKEN_ELEVATION>() as u32;

/// Whether this process runs elevated, from its access token.
///
/// # Errors
/// The token cannot be opened or read. The caller must not take that for
/// "not elevated".
#[allow(unsafe_code)]
pub fn is_elevated() -> io::Result<bool> {
    let token = process_token()?;
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut written = 0u32;
    // SAFETY: `token` is an open token handle with query access, alive for
    // the call. `elevation` is a live, writable `TOKEN_ELEVATION` whose size
    // is the one given, and `written` a live, writable `u32`.
    let read = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&raw mut elevation).cast(),
            ELEVATION_SIZE,
            &mut written,
        )
    };
    if read == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(elevation.TokenIsElevated != 0)
}

/// This process's access token, opened for reading; closed on drop.
#[allow(unsafe_code)]
fn process_token() -> io::Result<OwnedHandle> {
    let mut handle = null_mut();
    // SAFETY: `GetCurrentProcess` returns a pseudo handle that needs no
    // closing and is valid for the whole process. `handle` is a live,
    // writable handle for the call to fill.
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) };
    if opened == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success `handle` is a new token handle that nothing else
    // owns; `OwnedHandle` closes it once.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_process_token_answers() {
        // Hosted CI runners run elevated, the test VM's user does not: the
        // answer differs, but there is one.
        assert!(is_elevated().is_ok());
    }
}
