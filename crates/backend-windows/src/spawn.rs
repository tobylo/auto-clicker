//! Starting a program that inherits exactly the handles it is given
//! (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`): the daemon from its watchdog
//! (decision 6), and the settings window from the daemon (decision 14).
//! Every other inheritable handle of this process stays out of the child.

use std::ffi::{OsStr, OsString};
use std::io;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    UpdateProcThreadAttribute,
};

/// What the child's standard handles are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Standard {
    /// As Windows gives a child that names none.
    Kept,
    /// None: whatever the child or a library it loads prints goes nowhere,
    /// and nothing it reads comes from anywhere.
    Null,
}

/// Starts `program` with `arguments`, letting it inherit `inherited` and no
/// other handle, and returns its process handle. Each handle in
/// `inherited` must be inheritable, and stays open until this returns.
///
/// # Errors
/// The list of inherited handles could not be made, or the program could
/// not be started.
#[allow(unsafe_code)]
pub(crate) fn spawn(
    program: &Path,
    arguments: &[OsString],
    inherited: &[HANDLE],
    standard: Standard,
) -> io::Result<OwnedHandle> {
    let mut line = command_line(program.as_os_str(), arguments);
    let program = wide_path(program.as_os_str());
    let mut list = AttributeList::with_handles(inherited)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>()).unwrap_or(u32::MAX);
    startup.lpAttributeList = list.as_mut_ptr();
    if standard == Standard::Null {
        // The three standard handles stay null.
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    }
    let mut started = PROCESS_INFORMATION::default();
    // SAFETY: `program` and `line` are NUL-terminated UTF-16, `line`
    // writable as Windows requires, both alive for the call; inheritance is
    // on, limited by the attribute list to the handles in `inherited`,
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
    Ok(unsafe { OwnedHandle::from_raw_handle(started.hProcess) })
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
        let mut built = AttributeList {
            memory,
            _handles: handles,
        };
        // SAFETY: the list was initialised above for one attribute; the
        // handles live in `built`, which outlives every use of the list.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                built.as_mut_ptr(),
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

    /// The list, for Windows to write: from the memory's mutable pointer.
    fn as_mut_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.memory.as_mut_ptr().cast()
    }
}

impl Drop for AttributeList {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the list was initialised and is deleted once.
        unsafe { DeleteProcThreadAttributeList(self.as_mut_ptr()) };
    }
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
