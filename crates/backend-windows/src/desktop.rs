//! A driver for the device tests, in the WN01 VM's desktop: the pointer,
//! the keyboard, the tray icon, its popup and its menu, seen and used from
//! outside, as a user's hand would, and a window to put in front. Built for tests alone (the `desktop` feature), so the
//! daemon's tests drive Windows without `unsafe` of their own.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::FileExt;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOD_CONTROL, MOD_SHIFT, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEINPUT, RegisterHotKey, SendInput, UnregisterHotKey, VIRTUAL_KEY,
};
use windows_sys::Win32::UI::Shell::{NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, FindWindowW, GetMenuItemCount, GetMenuItemRect,
    GetMenuState, GetMenuStringW, GetSystemMetrics, GetWindowRect, HMENU, IsWindowVisible,
    MF_BYPOSITION, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MN_GETHMENU, SM_CXSCREEN,
    SM_CYSCREEN, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETTEXT, WindowFromPoint,
};

use crate::message::wide;

mod front;
mod screen;
mod windows;

pub use front::Front;
pub use screen::{DisplayMode, display_sizes, pointer, screen_size};
pub use windows::{Window, windows_of};

/// A rectangle on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Area {
    pub fn centre(self) -> Point {
        Point {
            x: self.left + (self.right - self.left) / 2,
            y: self.top + (self.bottom - self.top) / 2,
        }
    }
}

/// A point on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    /// The point `by` pixels further down (up when negative).
    pub fn down(self, by: i32) -> Point {
        Point {
            y: self.y + by,
            ..self
        }
    }
}

/// A process, by its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Process(pub u32);

impl Process {
    /// The process running this code.
    pub fn this() -> Process {
        Process(std::process::id())
    }
}

impl From<RECT> for Area {
    fn from(rect: RECT) -> Area {
        Area {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

/// The message threads' windows of process `pid`, among the first few it
/// made.
fn message_windows(Process(pid): Process) -> impl Iterator<Item = HWND> {
    (0..64).filter_map(move |n| window(&format!("auto-clicker.message-window.{pid}.{n}")))
}

/// The tray icon of process `pid` on the screen, where the taskbar shows
/// it (the overflow's button while it is hidden there); none while there
/// is no icon.
pub fn tray_icon(process: Process) -> Option<Area> {
    icon_owner(process)?.area()
}

/// The window that owns process `pid`'s tray icon, while it has one.
pub fn icon_owner(process: Process) -> Option<IconOwner> {
    message_windows(process)
        .map(IconOwner)
        .find(|owner| owner.area().is_some())
}

/// A tray icon's owner, by its window: still asked after the window is
/// gone, so an icon left behind is seen.
#[derive(Clone, Copy, Debug)]
pub struct IconOwner(HWND);

impl IconOwner {
    /// Where the taskbar shows the icon; none once it has no such icon.
    #[allow(unsafe_code)]
    pub fn area(self) -> Option<Area> {
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.0,
            uID: 1,
            ..NOTIFYICONIDENTIFIER::default()
        };
        let mut rect = RECT::default();
        // SAFETY: both are live and fully set for the call; the window is
        // only named, never used.
        let result = unsafe { Shell_NotifyIconGetRect(&identifier, &mut rect) };
        (result == 0).then(|| Area::from(rect))
    }
}

/// The text of process `pid`'s tray popup while it shows.
pub fn popup_text(process: Process) -> Option<String> {
    shown_popup(process).map(text)
}

/// Whether process `pid`'s tray popup, while it shows, is the window a
/// click on its middle would reach, as this thread sees it.
#[allow(unsafe_code)]
pub fn popup_takes_clicks(process: Process) -> Option<bool> {
    let popup = shown_popup(process)?;
    let mut rect = RECT::default();
    // SAFETY: any handle may be asked; `rect` is live.
    if unsafe { GetWindowRect(popup, &mut rect) } == 0 {
        return None;
    }
    let Point { x, y } = Area::from(rect).centre();
    // SAFETY: a point alone.
    let hit = unsafe { WindowFromPoint(POINT { x, y }) };
    Some(hit == popup)
}

fn shown_popup(Process(pid): Process) -> Option<HWND> {
    (0..64)
        .filter_map(|n| window(&format!("auto-clicker.tray-popup.{pid}.{n}")))
        .find(|&popup| visible(popup))
}

/// Moves the pointer to `to` as a mouse would, through the steps a mouse
/// takes on its way, so that what is under it hears it arrive.
pub fn move_pointer(to: Point) -> io::Result<()> {
    for step in [-24, -12, -4, 0] {
        let on_the_way = Point {
            x: to.x + step,
            ..to
        };
        send_mouse(on_the_way, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE)?;
    }
    Ok(())
}

/// Which button a click uses.
#[derive(Clone, Copy, Debug)]
pub enum Button {
    Left,
    Right,
}

/// Clicks at `at`, and lets the program clicked bring a window to the
/// front, as a real click does.
#[allow(unsafe_code)]
pub fn click(at: Point, button: Button) -> io::Result<()> {
    let (down, up) = match button {
        Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
    };
    // SAFETY: no arguments but a constant.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    move_pointer(at)?;
    send_mouse(at, MOUSEEVENTF_ABSOLUTE | down)?;
    send_mouse(at, MOUSEEVENTF_ABSOLUTE | up)
}

/// Sends the key with set-1 scan code `scan` (`extended` for an `E0` key)
/// down or up, as a keyboard does: what raw input and hotkeys see.
pub fn key(scan: u16, extended: bool, down: bool) -> io::Result<()> {
    send(&key_input(scan, extended, down))
}

/// Sends these keys (not extended) down or up in one go, as a macro pad
/// does: they are all queued before anything reads the first.
#[allow(unsafe_code)]
pub fn keys(keys: &[(u16, bool)]) -> io::Result<()> {
    let inputs: Vec<INPUT> = keys
        .iter()
        .map(|&(scan, down)| key_input(scan, false, down))
        .collect();
    let count = u32::try_from(inputs.len()).map_err(io::Error::other)?;
    let size = i32::try_from(size_of::<INPUT>()).map_err(io::Error::other)?;
    // SAFETY: `count` fully set `INPUT`s of their own size, alive for the
    // call.
    if unsafe { SendInput(count, inputs.as_ptr(), size) } != count {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn key_input(scan: u16, extended: bool, down: bool) -> INPUT {
    let extended = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
    let up = if down { 0 } else { KEYEVENTF_KEYUP };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wScan: scan,
                dwFlags: KEYEVENTF_SCANCODE | extended | up,
                ..KEYBDINPUT::default()
            },
        },
    }
}

/// Ctrl+Shift and a key (a virtual key code), registered as a hotkey of
/// this thread, as another program would hold it, until dropped on this
/// thread.
pub struct HeldHotkey(i32);

/// Gives each held hotkey its id.
static HOTKEYS: AtomicU64 = AtomicU64::new(0xA000);

impl HeldHotkey {
    /// Takes Ctrl+Shift+`key`.
    ///
    /// # Errors
    /// Another program holds it already.
    #[allow(unsafe_code)]
    pub fn hold(key: VIRTUAL_KEY) -> io::Result<HeldHotkey> {
        let id =
            i32::try_from(HOTKEYS.fetch_add(1, Ordering::Relaxed)).map_err(io::Error::other)?;
        // SAFETY: a hotkey of this thread, no window.
        if unsafe { RegisterHotKey(null_mut(), id, MOD_CONTROL | MOD_SHIFT, u32::from(key)) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(HeldHotkey(id))
    }

    /// Whether another program holds Ctrl+Shift+`key`: this thread cannot
    /// take it.
    pub fn taken(key: VIRTUAL_KEY) -> bool {
        HeldHotkey::hold(key).is_err()
    }
}

impl Drop for HeldHotkey {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: registered by this thread with this id.
        unsafe { UnregisterHotKey(null_mut(), self.0) };
    }
}

/// Where `window` is on the screen, if it is a window.
#[allow(unsafe_code)]
fn area_of(window: HWND) -> Option<Area> {
    let mut rect = RECT::default();
    // SAFETY: any handle may be asked; `rect` is live.
    (unsafe { GetWindowRect(window, &mut rect) } != 0).then(|| Area::from(rect))
}

/// Locks the session, as Windows+L does; it is unlocked by hand (in the
/// VM, through the QEMU monitor: see `tests/windows_daemon.rs`).
///
/// # Errors
/// Windows refused.
pub fn lock() -> io::Result<()> {
    // Windows' own entry point, through the program it ships for that.
    let status = Command::new("rundll32.exe")
        .arg("user32.dll,LockWorkStation")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!("rundll32 ended with {status}")));
    }
    Ok(())
}

/// Presses and releases the key `key` (a virtual key code).
pub fn press(key: VIRTUAL_KEY) -> io::Result<()> {
    send_key(key, 0)?;
    send_key(key, KEYEVENTF_KEYUP)
}

/// Whether the key or button `key` (a virtual key code) reads as down. Every
/// key reads up while a window of a higher integrity is in front.
#[allow(unsafe_code)]
pub fn is_down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: plain query.
    let state = unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(i32::from(key))
    };
    state.cast_unsigned() & 0x8000 != 0
}

/// A process held by its handle, so its id cannot pass to another process
/// while it is held; ended when dropped, unless it already ended or was let
/// go.
pub struct Held {
    handle: std::os::windows::io::OwnedHandle,
    armed: bool,
}

impl Held {
    /// Holds process `pid`, with the right to end it and wait on it.
    ///
    /// # Errors
    /// No such process, or it cannot be opened.
    #[allow(unsafe_code)]
    pub fn open(Process(pid): Process) -> io::Result<Held> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };
        let access = PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
        // SAFETY: plain call; the handle, if any, is owned below.
        let process = unsafe { OpenProcess(access, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a new handle that nothing else owns.
        let handle = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(process) };
        Ok(Held {
            handle,
            armed: true,
        })
    }

    /// Holds process `pid` only while `still` says, once it is held, that
    /// the id still names the process meant: an id read before it was held
    /// may by then be another process's. That other process is let go, not
    /// ended.
    ///
    /// # Errors
    /// No such process, it cannot be opened, or it is not the one meant.
    pub fn open_checked(pid: Process, still: impl FnOnce() -> bool) -> io::Result<Held> {
        let mut held = Held::open(pid)?;
        // Let go until the check says otherwise, a check that panics
        // included.
        held.armed = false;
        if !still() {
            return Err(io::Error::other("the process id passed to another process"));
        }
        held.armed = true;
        Ok(held)
    }
}

impl Drop for Held {
    /// Ends the process at once, as Task Manager's "End task" does, and
    /// waits up to five seconds for it to be gone.
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        if !self.armed {
            return;
        }
        use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
        // SAFETY: a live process handle with the right to end it; one that
        // already ended refuses, harmlessly.
        unsafe { TerminateProcess(self.handle.as_raw_handle(), 1) };
        // SAFETY: as above, with the right to wait on it.
        unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 5_000) };
    }
}

/// Whether process `pid` is per-monitor DPI aware (version 2); `None` when
/// it cannot be asked.
#[allow(unsafe_code)]
pub fn per_monitor_aware(Process(pid): Process) -> Option<bool> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::HiDpi::{
        AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        GetDpiAwarenessContextForProcess,
    };
    // SAFETY: plain call; the handle, if any, is owned below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    // SAFETY: a new handle that nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    // SAFETY: a live process handle with limited query access; the
    // contexts are values, not pointers this program follows.
    Some(
        unsafe {
            AreDpiAwarenessContextsEqual(
                GetDpiAwarenessContextForProcess(process.as_raw_handle()),
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            )
        } != 0,
    )
}

/// The menu that shows, if one does; none when its thread does not answer
/// within half a second.
#[allow(unsafe_code)]
pub fn shown_menu() -> Option<Menu> {
    let window = window("#32768").filter(|&menu| visible(menu))?;
    let mut menu = 0;
    // SAFETY: `window` is a menu window; the message asks for its menu,
    // written to `menu`, live for the call.
    let answered =
        unsafe { SendMessageTimeoutW(window, MN_GETHMENU, 0, 0, SMTO_ABORTIFHUNG, 500, &mut menu) };
    (answered != 0 && menu != 0).then_some(Menu(menu as HMENU))
}

/// A line of a menu, as it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub label: String,
    pub greyed: bool,
    pub checked: bool,
    pub submenu: bool,
    pub separator: bool,
}

/// A menu that shows: another process's, read and clicked from outside.
pub struct Menu(HMENU);

impl Menu {
    /// Its lines, in order.
    #[allow(unsafe_code)]
    pub fn lines(&self) -> Vec<Line> {
        // SAFETY: a menu handle is valid in every process of the session;
        // reading a stale one only fails.
        let count = unsafe { GetMenuItemCount(self.0) };
        (0..u32::try_from(count).unwrap_or(0))
            .map(|place| self.line(place))
            .collect()
    }

    #[allow(unsafe_code)]
    fn line(&self, place: u32) -> Line {
        let mut label = [0u16; 256];
        // SAFETY: `label` is a live buffer of the length given.
        let length =
            unsafe { GetMenuStringW(self.0, place, label.as_mut_ptr(), 256, MF_BYPOSITION) };
        let label = String::from_utf16_lossy(&label[..usize::try_from(length).unwrap_or(0)]);
        // SAFETY: as above.
        let state = unsafe { GetMenuState(self.0, place, MF_BYPOSITION) };
        Line {
            label,
            greyed: state & MF_GRAYED != 0,
            checked: state & MF_CHECKED != 0,
            submenu: state & MF_POPUP != 0,
            // A submenu's state holds its count in the high byte, where
            // eight items would read as a separator.
            separator: state & MF_POPUP == 0 && state & MF_SEPARATOR != 0,
        }
    }

    /// Where the line at `place` shows.
    #[allow(unsafe_code)]
    pub fn area(&self, place: u32) -> Option<Area> {
        let mut rect = RECT::default();
        // SAFETY: no window: the menu's own is found; `rect` is live.
        let found = unsafe { GetMenuItemRect(null_mut(), self.0, place, &mut rect) };
        (found != 0).then(|| Area::from(rect))
    }

    /// Clicks the line at `place`.
    pub fn click(&self, place: u32) -> io::Result<()> {
        let area = self
            .area(place)
            .ok_or_else(|| io::Error::other(format!("no line {place}")))?;
        click(area.centre(), Button::Left)
    }
}

#[allow(unsafe_code)]
fn window(class: &str) -> Option<HWND> {
    let class = wide(class);
    // SAFETY: `class` is NUL-terminated UTF-16, alive for the call.
    let window = unsafe { FindWindowW(class.as_ptr(), null()) };
    (!window.is_null()).then_some(window)
}

#[allow(unsafe_code)]
fn visible(window: HWND) -> bool {
    // SAFETY: any handle may be asked; a stale one is not visible.
    unsafe { IsWindowVisible(window) != 0 }
}

/// `window`'s text; empty when its thread does not answer within half a
/// second, which a window of this process would otherwise make wait.
#[allow(unsafe_code)]
fn text(window: HWND) -> String {
    let mut text = vec![0u16; 4096];
    let mut length = 0;
    // SAFETY: `text` is a live buffer of the length given; `length` is
    // live. A send that timed out may still be handled after the call
    // returns, so the buffer is then never freed (below).
    let answered = unsafe {
        SendMessageTimeoutW(
            window,
            WM_GETTEXT,
            text.len(),
            text.as_mut_ptr() as isize,
            SMTO_ABORTIFHUNG,
            500,
            &mut length,
        )
    };
    if answered == 0 {
        // The window's thread may write into it yet: it is leaked, a
        // test's few kilobytes, rather than freed under that write.
        std::mem::forget(text);
        return String::new();
    }
    String::from_utf16_lossy(&text[..length.min(text.len())])
}

/// Runs `script` in PowerShell, waiting at most `within`; its output. One
/// that runs past its deadline is ended. Its output goes to files, not
/// pipes, so nothing waits on reading it, nor on a program the script
/// started that still holds it.
///
/// # Errors
/// PowerShell could not start, ran past its deadline, or failed, or its
/// output could not be read.
pub fn powershell(script: &str, within: Duration) -> io::Result<String> {
    let deadline = Instant::now() + within;
    let stdout = Output::new("out")?;
    let stderr = Output::new("err")?;
    let mut child = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::null())
        .stdout(stdout.file()?)
        .stderr(stderr.file()?)
        .spawn()?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{script}: past {within:?}"),
            ));
        }
        sleep(Duration::from_millis(50));
    };
    succeeded(status, script, &stderr.read()?)?;
    stdout.read()
}

/// A file a child writes its output to, made afresh (never one that was
/// there) and removed when dropped.
struct Output {
    path: PathBuf,
    file: File,
}

impl Output {
    /// The next free name for `kind`: one a run that ended without
    /// cleaning up left behind is passed over, never opened.
    fn new(kind: &str) -> io::Result<Output> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let mut left_behind = None;
        for _ in 0..64 {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let name = format!("auto-clicker-desktop.{}.{n}.{kind}", std::process::id());
            let path = std::env::temp_dir().join(name);
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => return Ok(Output { path, file }),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => left_behind = Some(err),
                Err(err) => return Err(err),
            }
        }
        Err(left_behind.unwrap_or_else(|| io::Error::other("no free name")))
    }

    /// The file for the child to write to: the one made here.
    fn file(&self) -> io::Result<File> {
        self.file.try_clone()
    }

    /// What it holds now, read through the file made here: a program the
    /// script started may write on, and is not waited for.
    fn read(&self) -> io::Result<String> {
        let length = usize::try_from(self.file.metadata()?.len()).map_err(io::Error::other)?;
        let mut read = vec![0; length];
        let mut done = 0;
        while done < length {
            let offset = u64::try_from(done).map_err(io::Error::other)?;
            match self.file.seek_read(&mut read[done..], offset)? {
                0 => break,
                count => done += count,
            }
        }
        read.truncate(done);
        Ok(String::from_utf8_lossy(&read).into_owned())
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn succeeded(status: ExitStatus, script: &str, stderr: &str) -> io::Result<()> {
    if status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!("{script}: {status}: {stderr}")))
}

#[allow(unsafe_code)]
fn send_mouse(at: Point, flags: u32) -> io::Result<()> {
    // SAFETY: the metrics' indices alone.
    let (width, height) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let normal = |at: i32, size: i32| at * 65535 / (size - 1).max(1);
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: normal(at.x, width),
                dy: normal(at.y, height),
                dwFlags: flags,
                ..MOUSEINPUT::default()
            },
        },
    };
    send(&input)
}

fn send_key(key: VIRTUAL_KEY, flags: u32) -> io::Result<()> {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: flags,
                ..KEYBDINPUT::default()
            },
        },
    };
    send(&input)
}

#[allow(unsafe_code)]
fn send(input: &INPUT) -> io::Result<()> {
    // SAFETY: one fully set `INPUT` of its own size.
    let sent = unsafe { SendInput(1, input, size_of::<INPUT>() as i32) };
    if sent != 1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_left_behind_is_passed_over_and_kept() {
        let first = Output::new("left").unwrap();
        // The name `by` after the first: `<prefix>.<pid>.<n>.left`.
        let name = first
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let parts: Vec<&str> = name.split('.').collect();
        let at: u64 = parts[2].parse().unwrap();
        let next = |by: u64| {
            let name = format!("{}.{}.{}.left", parts[0], parts[1], at + by);
            first.path.with_file_name(name)
        };
        // What a run that crashed left at the next two names; made new, so
        // nothing that was there is touched.
        for by in [1, 2] {
            let mut left = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(next(by))
                .unwrap();
            std::io::Write::write_all(&mut left, b"kept").unwrap();
        }
        let made = Output::new("left").unwrap();
        assert!(made.read().unwrap().is_empty());
        for by in [1, 2] {
            assert_eq!(std::fs::read_to_string(next(by)).unwrap(), "kept");
            std::fs::remove_file(next(by)).unwrap();
        }
    }

    #[test]
    fn powershell_gives_all_its_output_and_keeps_its_deadline() {
        // More than a pipe holds, read while it runs.
        let big = powershell("'x' * 2097152", Duration::from_secs(60)).unwrap();
        assert_eq!(big.trim_end().len(), 2_097_152);
        let failed = powershell("exit 3", Duration::from_secs(60)).unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::Other);
        // Past its deadline: ended.
        let asked = Instant::now();
        let late = powershell("Start-Sleep -Seconds 60", Duration::from_secs(5)).unwrap_err();
        assert_eq!(late.kind(), io::ErrorKind::TimedOut);
        assert!(asked.elapsed() < Duration::from_secs(30));
        // A program it started still writes to its output: it is done when
        // it ends, with what was there then.
        for started in [
            "Start-Process ping -ArgumentList '-n 60 127.0.0.1' -NoNewWindow",
            "Start-Process powershell -ArgumentList '-Command', 'for ($i = 0; $i -lt 600; $i++) { \"x\" * 4096; Start-Sleep -Milliseconds 50 }' \
             -NoNewWindow; Start-Sleep -Seconds 1",
        ] {
            let asked = Instant::now();
            powershell(started, Duration::from_secs(30)).unwrap();
            assert!(asked.elapsed() < Duration::from_secs(20), "{started}");
        }
    }
}
