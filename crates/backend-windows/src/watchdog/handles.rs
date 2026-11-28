//! The two handles the watchdog gives its daemon (M5 brief decision 6):
//! the table's section and a handle to the watchdog's own process, which
//! the daemon waits on. They pass by inheritance, never by name or process
//! id: the watchdog starts the daemon with exactly these two in its list of
//! inherited handles, and the daemon takes them on, checks each and makes
//! sure no child of its own inherits them.

use std::ffi::OsString;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessId, INFINITE,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

use super::table::{Section, Table, map};
use crate::spawn::{Standard, spawn};

/// The values of the two handles a daemon inherits, as its command line
/// names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handles {
    /// The table's section.
    pub table: usize,
    /// The watchdog's process: SYNCHRONIZE, to wait on it, and limited
    /// query, to check that it is a process at all.
    pub watchdog: usize,
}

/// A daemon the watchdog started.
pub struct Daemon {
    process: OwnedHandle,
}

impl Daemon {
    /// Waits until the daemon ends, however long that is, and returns its
    /// exit code. The watchdog does nothing else while its daemon lives.
    ///
    /// # Errors
    /// Windows could not wait on the process or read its exit code.
    #[allow(unsafe_code)]
    pub fn wait(&self) -> io::Result<u32> {
        // SAFETY: `process` is this daemon's live process handle, with the
        // full access `CreateProcessW` gives its creator.
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        let mut code = 0u32;
        // SAFETY: as above; `code` is live and writable.
        if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(code)
    }

    /// The daemon's process id, for the log.
    #[allow(unsafe_code)]
    pub fn id(&self) -> u32 {
        // SAFETY: a live process handle with full access.
        unsafe { GetProcessId(self.process.as_raw_handle()) }
    }
}

/// Starts `program` with the arguments `arguments` makes from the handles'
/// values, letting it inherit the table's section and a handle to this
/// process, and nothing else.
///
/// # Errors
/// The handle to this process or the list of inherited handles could not be
/// made, or the program could not be started.
pub fn start_daemon(
    program: &Path,
    section: &Section,
    arguments: impl FnOnce(Handles) -> Vec<OsString>,
) -> io::Result<Daemon> {
    let watchdog = own_process()?;
    let inherited = [section.raw(), watchdog.as_raw_handle()];
    let handles = Handles {
        table: inherited[0] as usize,
        watchdog: inherited[1] as usize,
    };
    let process = spawn(program, &arguments(handles), &inherited, Standard::Kept)?;
    // The daemon holds its own copy of `watchdog` now; this one goes when
    // dropped.
    drop(watchdog);
    Ok(Daemon { process })
}

/// An inheritable handle to this process that can only be waited on and
/// asked its id.
#[allow(unsafe_code)]
fn own_process() -> io::Result<OwnedHandle> {
    let mut duplicate: HANDLE = null_mut();
    // SAFETY: `GetCurrentProcess` is a pseudo-handle needing no closing;
    // `duplicate` is live and writable.
    let made = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            GetCurrentProcess(),
            GetCurrentProcess(),
            &mut duplicate,
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            1,
            0,
        )
    };
    if made == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a new handle that nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(duplicate) })
}

/// What the daemon takes on from its watchdog: the table, mapped, and the
/// watchdog's process, to wait on.
pub struct Attached {
    pub table: Table,
    pub watchdog: OwnedHandle,
}

/// Takes on the handles named `handles`, inherited from the watchdog: maps
/// the section at the table's size and checks that the other is a process
/// (`GetProcessId` succeeds). The section's handle is closed once mapped,
/// and the process handle is made non-inheritable, so no child of the
/// daemon inherits either.
///
/// A value that names no such handle fails, and nothing is closed then: it
/// may name a handle that is not the daemon's to close. The daemon refuses
/// to run and exits.
///
/// # Errors
/// Either handle is not what it should be, or its inheritance could not be
/// turned off.
#[allow(unsafe_code)]
pub fn attach(handles: Handles) -> io::Result<Attached> {
    let section = handles.table as HANDLE;
    let watchdog = handles.watchdog as HANDLE;
    let table = map(section).map_err(|err| context(err, "the table cannot be mapped"))?;
    // SAFETY: any value may be asked; one that is no process handle with
    // query access fails.
    if unsafe { GetProcessId(watchdog) } == 0 {
        return Err(context(
            io::Error::last_os_error(),
            "the watchdog's handle names no process",
        ));
    }
    // SAFETY: `watchdog` was checked to be a process handle of this process.
    if unsafe { SetHandleInformation(watchdog, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(context(
            io::Error::last_os_error(),
            "the watchdog's handle cannot be kept from children",
        ));
    }
    // SAFETY: the section was inherited for this process alone, and its
    // view keeps the section alive; the handle is closed once.
    unsafe { CloseHandle(section) };
    // SAFETY: inherited for this process alone and owned from here on.
    let watchdog = unsafe { OwnedHandle::from_raw_handle(watchdog) };
    Ok(Attached { table, watchdog })
}

fn context(err: io::Error, what: &str) -> io::Error {
    io::Error::new(err.kind(), format!("{what}: {err}"))
}

#[cfg(test)]
mod tests;
