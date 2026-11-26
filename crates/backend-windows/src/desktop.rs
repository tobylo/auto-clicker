//! A driver for the device tests, in the WN01 VM's desktop: the pointer,
//! the tray icon, its popup and its menu, seen and used from outside, as a
//! user's hand would. Built for tests alone (the `desktop` feature), so the
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
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEINPUT, SendInput, VIRTUAL_KEY,
};
use windows_sys::Win32::UI::Shell::{NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, FindWindowW, GetMenuItemCount, GetMenuItemRect,
    GetMenuState, GetMenuStringW, GetSystemMetrics, GetWindowRect, HMENU, IsWindowVisible,
    MF_BYPOSITION, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MN_GETHMENU, SM_CXSCREEN,
    SM_CYSCREEN, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETTEXT, WindowFromPoint,
};

use crate::message::wide;

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
