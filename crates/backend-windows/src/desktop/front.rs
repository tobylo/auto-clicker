//! A window of the test's own in front, a stand-in for a game: plain, or
//! reading the keyboard in raw input with `RIDEV_NOHOTKEYS`, which turns
//! every registered hotkey off while it has focus (0006 point 5).

use std::io;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::{RAWINPUTDEVICE, RIDEV_NOHOTKEYS, RegisterRawInputDevices};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    GetMessageW, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOW, ShowWindow,
    UnregisterClassW, WM_APP, WM_DESTROY, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use super::Area;
use crate::message::wide;

/// Asks the window to close.
const WM_CLOSE_FRONT: u32 = WM_APP + 1;

/// Names each window's class apart.
static CLASSES: AtomicU64 = AtomicU64::new(0);

/// The window, on a thread of its own, until dropped.
pub struct Front {
    window: usize,
    thread: Option<JoinHandle<()>>,
}

impl Front {
    /// Opens a window of 640 by 480 at the top left, reading the keyboard
    /// with `RIDEV_NOHOTKEYS` when `no_hotkeys`. Click it to bring it in
    /// front.
    ///
    /// # Errors
    /// The window or its registration could not be made.
    pub fn open(no_hotkeys: bool) -> io::Result<Front> {
        // A rendezvous: a window sent is one the opener took.
        let (ready, made) = mpsc::sync_channel(0);
        let thread = thread::spawn(move || serve(no_hotkeys, &ready));
        let window = made
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| io::Error::other("the window was not made in time"))??;
        Ok(Front {
            window,
            thread: Some(thread),
        })
    }

    /// Where it is on the screen.
    pub fn area(&self) -> Area {
        super::area_of(self.hwnd()).unwrap_or(Area {
            left: 0,
            top: 0,
            right: 640,
            bottom: 480,
        })
    }

    /// Whether it is the window in front.
    #[allow(unsafe_code)]
    pub fn in_front(&self) -> bool {
        // SAFETY: no arguments.
        unsafe { GetForegroundWindow() == self.hwnd() }
    }

    fn hwnd(&self) -> HWND {
        self.window as HWND
    }
}

impl Drop for Front {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the window's thread destroys it on this message; a window
        // already gone only makes the post fail.
        let posted = unsafe { PostMessageW(self.hwnd(), WM_CLOSE_FRONT, 0, 0) } != 0;
        // A thread that was not told to end is left to the process's end,
        // rather than waited for forever; one whose window is gone has ended.
        if let Some(thread) = self
            .thread
            .take()
            .filter(|thread| posted || thread.is_finished())
        {
            let _ = thread.join();
        }
    }
}

/// The window's thread: makes it, says so, and runs its messages.
#[allow(unsafe_code)]
fn serve(no_hotkeys: bool, ready: &mpsc::SyncSender<io::Result<usize>>) {
    let class = wide(&format!(
        "auto-clicker-test.front.{}.{}",
        std::process::id(),
        CLASSES.fetch_add(1, Ordering::Relaxed)
    ));
    let made = make(&class, no_hotkeys);
    let window = made.as_ref().ok().copied();
    // An opener that stopped waiting has no `Front` to close the window.
    let heard = ready.send(made.map(|hwnd| hwnd as usize)).is_ok();
    if let Some(hwnd) = window.filter(|_| !heard) {
        // SAFETY: this thread's window, which no one else knows of.
        unsafe { DestroyWindow(hwnd) };
    }
    if window.is_some() && heard {
        let mut message = MSG::default();
        // SAFETY: `message` is a live `MSG`; every message of this thread.
        while unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } > 0 {
            // SAFETY: just filled in by `GetMessageW`.
            unsafe { DispatchMessageW(&message) };
        }
    }
    // SAFETY: the class registered by `make`, its window gone.
    unsafe { UnregisterClassW(class.as_ptr(), GetModuleHandleW(null())) };
}

#[allow(unsafe_code)]
fn make(class: &[u16], no_hotkeys: bool) -> io::Result<HWND> {
    // SAFETY: this program's own module.
    let instance = unsafe { GetModuleHandleW(null()) };
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        ..WNDCLASSW::default()
    };
    // SAFETY: fully set; the name outlives the class.
    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let title = wide("Auto Clicker test window");
    // SAFETY: the class above; the title alive for the call.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            0,
            0,
            640,
            480,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: this thread's window.
    unsafe { ShowWindow(hwnd, SW_SHOW) };
    if no_hotkeys {
        let device = RAWINPUTDEVICE {
            usUsagePage: 1,
            usUsage: 6,
            dwFlags: RIDEV_NOHOTKEYS,
            hwndTarget: hwnd,
        };
        let size = u32::try_from(size_of::<RAWINPUTDEVICE>()).unwrap_or(u32::MAX);
        // SAFETY: one device, fully set, alive for the call.
        if unsafe { RegisterRawInputDevices(&device, 1, size) } == 0 {
            let err = io::Error::last_os_error();
            // SAFETY: this thread's window.
            unsafe { DestroyWindow(hwnd) };
            return Err(err);
        }
    }
    Ok(hwnd)
}

#[allow(unsafe_code)]
unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CLOSE_FRONT => {
            // SAFETY: this thread's window.
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            // SAFETY: no arguments but the exit code.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: what Windows passed in.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
