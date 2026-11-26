//! The two handles the watchdog gives its daemon (M5 brief decision 6):
//! the table's section and a handle to the watchdog's own process, which
//! the daemon waits on. They pass by inheritance, never by name or process
//! id: the watchdog starts the daemon with exactly these two in its list of
//! inherited handles, and the daemon takes them on, checks each and makes
//! sure no child of its own inherits them.

use std::ffi::{OsStr, OsString};
use std::io;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, GetProcessId, INFINITE, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, STARTUPINFOEXW,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::table::{Section, Table, map};

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
#[allow(unsafe_code)]
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
    let mut line = command_line(program.as_os_str(), &arguments(handles));
    let program = wide_path(program.as_os_str());
    let list = AttributeList::with_handles(&inherited)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>()).unwrap_or(u32::MAX);
    startup.lpAttributeList = list.as_ptr();
    let mut started = PROCESS_INFORMATION::default();
    // SAFETY: `program` and `line` are NUL-terminated UTF-16, `line`
    // writable as Windows requires, both alive for the call; inheritance is
    // on, limited by the attribute list to the two handles in `inherited`,
    // which stay open until the call returns; the environment and folder
    // are this process's; `startup` and `started` are live.
    let created = unsafe {
        CreateProcessW(
            program.as_ptr(),
            line.as_mut_ptr(),
            null(),
            null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT,
            null(),
            null(),
            &startup.StartupInfo,
            &mut started,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the thread handle is new and only closed here.
    unsafe { CloseHandle(started.hThread) };
    // SAFETY: the process handle is new and owned from here on.
    let process = unsafe { OwnedHandle::from_raw_handle(started.hProcess) };
    // The daemon holds its own copy of `watchdog` now; this one goes when
    // dropped, as does the list.
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

/// A list of attributes naming the handles a child inherits.
struct AttributeList {
    /// The list's memory, as `usize`s so it is aligned for pointers.
    memory: Vec<usize>,
    /// The handles named, kept where the list points until it is gone.
    _handles: Box<[HANDLE]>,
}

impl AttributeList {
    #[allow(unsafe_code)]
    fn with_handles(handles: &[HANDLE]) -> io::Result<AttributeList> {
        let mut bytes = 0usize;
        // SAFETY: a null list asks only for the size, written to `bytes`;
        // this call fails by design.
        unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes) };
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut memory = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let list: LPPROC_THREAD_ATTRIBUTE_LIST = memory.as_mut_ptr().cast();
        // SAFETY: `list` points at at least `bytes` writable, aligned bytes.
        if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut bytes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let handles: Box<[HANDLE]> = handles.into();
        let built = AttributeList {
            memory,
            _handles: handles,
        };
        // SAFETY: the list was initialised above for one attribute; the
        // handles live in `built`, which outlives every use of the list.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                built.as_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                built._handles.as_ptr().cast(),
                size_of_val(&*built._handles),
                null_mut(),
                null(),
            )
        };
        if updated == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(built)
    }

    fn as_ptr(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.memory.as_ptr().cast_mut().cast()
    }
}

impl Drop for AttributeList {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the list was initialised and is deleted once.
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
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

/// `path` as NUL-terminated UTF-16.
fn wide_path(path: &OsStr) -> Vec<u16> {
    path.encode_wide().chain(once(0)).collect()
}

/// The command line `CreateProcessW` is given: the program, quoted, then
/// each argument quoted as the C runtime reads it back. NUL-terminated.
fn command_line(program: &OsStr, arguments: &[OsString]) -> Vec<u16> {
    let mut line: Vec<u16> = Vec::new();
    line.push(u16::from(b'"'));
    line.extend(program.encode_wide());
    line.push(u16::from(b'"'));
    for argument in arguments {
        line.push(u16::from(b' '));
        quote(argument, &mut line);
    }
    line.push(0);
    line
}

/// `argument`, quoted when it needs to be, as the C runtime splits a
/// command line: backslashes count only before a quote.
fn quote(argument: &OsStr, line: &mut Vec<u16>) {
    let units: Vec<u16> = argument.encode_wide().collect();
    let plain = !units.is_empty()
        && !units.iter().any(|&unit| {
            unit == u16::from(b' ') || unit == u16::from(b'\t') || unit == u16::from(b'"')
        });
    if plain {
        line.extend(units);
        return;
    }
    line.push(u16::from(b'"'));
    let mut backslashes = 0usize;
    for unit in units {
        if unit == u16::from(b'\\') {
            backslashes += 1;
            continue;
        }
        let doubled = if unit == u16::from(b'"') {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        line.extend(std::iter::repeat_n(u16::from(b'\\'), doubled));
        backslashes = 0;
        line.push(unit);
    }
    line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2));
    line.push(u16::from(b'"'));
}

#[cfg(test)]
mod tests;
