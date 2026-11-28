//! The settings window's link to the daemon (M5 brief decision 14, amended
//! by WN10): two pipes, one each way, whose window ends the window inherits
//! and finds named on its command line. They are not its standard input
//! and output, which it starts without, so nothing a library prints ever
//! reaches the daemon as a line of the protocol.
//!
//! The daemon makes each pipe as a named pipe of one instance, local
//! clients only, with a name of its own, and opens the window's end itself
//! at once: should another program have connected first, that open fails
//! and no window starts. The window inherits those two ends and nothing
//! else (an explicit handle list, as the watchdog's daemon does), and the
//! daemon closes its copies of them, so a pipe ends when the window does.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::future::Future;
use std::io;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::pin::pin;
use std::ptr::null;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Waker};

use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use windows_sys::Win32::Foundation::{
    GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT, HWND, INVALID_HANDLE_VALUE,
    SetHandleInformation,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_TYPE_PIPE, GetFileType, OPEN_EXISTING, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT,
};
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;
use windows_sys::Win32::System::Threading::{GetProcessId, TerminateProcess};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
};

use crate::spawn::{Standard, spawn};

/// The flag before the value of the pipe the window writes its requests to.
const REQUESTS: &str = "--requests";
/// The flag before the value of the pipe the window reads updates from.
const UPDATES: &str = "--updates";

/// The values of the window's two pipe ends, as its command line names
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pipes {
    /// Where the window writes its requests.
    pub requests: usize,
    /// Where the window reads the daemon's updates.
    pub updates: usize,
}

/// The window's command line for `pipes`, after its program.
pub fn arguments(pipes: Pipes) -> Vec<OsString> {
    [
        REQUESTS.to_owned(),
        pipes.requests.to_string(),
        UPDATES.to_owned(),
        pipes.updates.to_string(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// The pipes a window's command line names: exactly [`arguments`]' form,
/// two distinct values that are not zero; anything else is refused, and
/// says why.
///
/// # Errors
/// The arguments are not in that form.
pub fn parse(mut arguments: impl Iterator<Item = OsString>) -> Result<Pipes, String> {
    let mut value = |flag: &str| {
        if arguments.next().as_deref() != Some(OsStr::new(flag)) {
            return Err(format!("{flag} is missing"));
        }
        // Only as `arguments` writes it: decimal, no sign, no leading zero.
        let text = arguments.next();
        text.as_deref()
            .and_then(OsStr::to_str)
            .and_then(|text| Some((text, text.parse::<usize>().ok()?)))
            .filter(|&(text, value)| value != 0 && value.to_string() == text)
            .map(|(_, value)| value)
            .ok_or_else(|| format!("{flag} names no handle"))
    };
    let requests = value(REQUESTS)?;
    let updates = value(UPDATES)?;
    if arguments.next().is_some() {
        return Err("it takes no more arguments".to_owned());
    }
    if requests == updates {
        return Err("both pipes are one handle".to_owned());
    }
    Ok(Pipes { requests, updates })
}

/// Takes on the pipes named `pipes`, inherited from the daemon: where the
/// window writes its requests, and where it reads updates. Each must be a
/// pipe; each is made non-inheritable, so nothing the window starts (a
/// browser for the About tab's link) holds the daemon's link open.
///
/// A value that names no pipe fails, and nothing is closed then: it may
/// name a handle that is not the window's to close.
///
/// # Errors
/// A handle is no pipe, or its inheritance could not be turned off.
#[allow(unsafe_code)]
pub fn take(pipes: Pipes) -> io::Result<(File, File)> {
    for (value, which) in [(pipes.requests, REQUESTS), (pipes.updates, UPDATES)] {
        let handle = value as HANDLE;
        // SAFETY: any value may be asked; one that names no handle is no
        // pipe.
        if unsafe { GetFileType(handle) } != FILE_TYPE_PIPE {
            return Err(io::Error::other(format!("{which} names no pipe")));
        }
        // SAFETY: `handle` was checked to be a pipe of this process.
        if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    // SAFETY: both were inherited for this process alone, checked above,
    // and are distinct (`parse`); each is owned from here on.
    let own = |value: usize| File::from(unsafe { OwnedHandle::from_raw_handle(value as HANDLE) });
    Ok((own(pipes.requests), own(pipes.updates)))
}

/// A settings window the daemon started: killed when dropped.
pub struct Window {
    process: OwnedHandle,
}

impl Window {
    /// The window's process id.
    #[allow(unsafe_code)]
    pub fn id(&self) -> u32 {
        // SAFETY: a live process handle with full access.
        unsafe { GetProcessId(self.process.as_raw_handle()) }
    }
}

/// Lets the window whose process is `id` come to the front, as the program
/// in front (the daemon, whose tray the user just used) may let another.
/// A refusal leaves the window where it is.
#[allow(unsafe_code)]
pub fn let_forward(id: u32) {
    // SAFETY: plain call; any id may be passed.
    unsafe { AllowSetForegroundWindow(id) };
}

impl Drop for Window {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the window's live process handle, with the full access
        // its creator has; one that already exited fails, which is fine.
        unsafe { TerminateProcess(self.process.as_raw_handle(), 1) };
    }
}

/// A window that started, and the daemon's ends of its pipes.
pub struct Started {
    pub window: Window,
    /// What the window asks.
    pub requests: NamedPipeServer,
    /// Where the daemon writes its updates.
    pub updates: NamedPipeServer,
}

/// Gives each pipe a name no other of this process had.
static PIPES: AtomicU64 = AtomicU64::new(0);

/// Starts `program` as the settings window, with its two pipes. Must be
/// called on a tokio runtime with its I/O driver.
///
/// # Errors
/// A pipe could not be made or opened (another program connected first),
/// or the program could not start; nothing is left of what was made.
pub fn start(program: &Path) -> io::Result<Started> {
    start_with(program, &[])
}

/// [`start`], with `before` on the command line ahead of the pipes.
fn start_with(program: &Path, before: &[OsString]) -> io::Result<Started> {
    let (requests, theirs_requests) = pipe(Way::FromWindow)?;
    let (updates, theirs_updates) = pipe(Way::ToWindow)?;
    let inherited = [
        theirs_requests.as_raw_handle(),
        theirs_updates.as_raw_handle(),
    ];
    let pipes = Pipes {
        requests: inherited[0] as usize,
        updates: inherited[1] as usize,
    };
    let line: Vec<OsString> = before.iter().cloned().chain(arguments(pipes)).collect();
    let process = spawn(program, &line, &inherited, Standard::Null)?;
    // The window holds its own copies now; with these gone, each pipe ends
    // when the window does.
    drop((theirs_requests, theirs_updates));
    Ok(Started {
        window: Window { process },
        requests,
        updates,
    })
}

/// Which way a pipe carries lines.
#[derive(Clone, Copy)]
enum Way {
    FromWindow,
    ToWindow,
}

/// A new pipe: the daemon's end, connected, and the window's, opened and
/// inheritable.
fn pipe(way: Way) -> io::Result<(NamedPipeServer, OwnedHandle)> {
    let name = pipe_name();
    let server = server(&name, way)?;
    let theirs = window_end(&name, way)?;
    connected(&server)?;
    Ok((server, theirs))
}

/// Completes the daemon's end's connection to the window's end, opened
/// already, so it reads and writes: the runtime starts reading a named
/// pipe only once it is connected. With the other end open, that finishes
/// at once; anything else is an error.
fn connected(server: &NamedPipeServer) -> io::Result<()> {
    let mut connect = pin!(server.connect());
    match connect
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => Err(io::Error::other(
            "a pipe of the settings window did not connect",
        )),
    }
}

/// The daemon's end of the pipe `name`: its one instance, for local
/// clients, carrying lines `way`.
fn server(name: &OsStr, way: Way) -> io::Result<NamedPipeServer> {
    let inbound = matches!(way, Way::FromWindow);
    ServerOptions::new()
        .first_pipe_instance(true)
        .max_instances(1)
        .reject_remote_clients(true)
        .pipe_mode(PipeMode::Byte)
        .access_inbound(inbound)
        .access_outbound(!inbound)
        .create(name)
}

/// The window's end of the pipe `name`, inheritable; refused when another
/// program connected to its one instance first.
#[allow(unsafe_code)]
fn window_end(name: &OsStr, way: Way) -> io::Result<OwnedHandle> {
    let access = match way {
        Way::FromWindow => GENERIC_WRITE,
        Way::ToWindow => GENERIC_READ,
    };
    let wide: Vec<u16> = name.encode_wide().chain(once(0)).collect();
    // SAFETY: `wide` is a NUL-terminated name alive for the call; no
    // sharing, default security, no template. Synchronous: the window reads
    // and writes it with plain calls. Identification only, so the daemon's
    // own server end could never act as it.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            0,
            null(),
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a new handle owned from here on.
    let theirs = unsafe { OwnedHandle::from_raw_handle(handle) };
    // SAFETY: the handle just opened; inheritable only for the window's
    // start, which names it in its list, and closed right after.
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(theirs)
}

/// A pipe name of this process's own: its id, a count, and the
/// performance counter, so a name another program guessed ahead is
/// unlikely, and one it took makes the start fail rather than connect.
#[allow(unsafe_code)]
fn pipe_name() -> OsString {
    let mut tick = 0i64;
    // SAFETY: `tick` is live and writable; this cannot fail on Windows XP
    // and later, and 0 is a fine value if it did.
    unsafe { QueryPerformanceCounter(&mut tick) };
    let count = PIPES.fetch_add(1, Ordering::Relaxed);
    OsString::from(format!(
        r"\\.\pipe\auto-clicker-settings-{}-{count}-{tick:x}",
        std::process::id()
    ))
}

/// Brings the settings window `hwnd` to the front, restored if it was
/// minimised, as far as the daemon let it ([`Window::let_forward`]).
#[allow(unsafe_code)]
pub fn bring_forward(hwnd: isize) {
    let hwnd = hwnd as HWND;
    // SAFETY: any value may be passed; one that is no window fails.
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
}

#[cfg(test)]
mod tests;
