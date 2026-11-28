//! Another program's windows, seen from outside: the settings window the
//! daemon starts, found by its process, its place on the screen, whether
//! it is in front, and asked to close as its close button does.

use std::io;

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetClientRect, GetForegroundWindow, GetWindow, GetWindowRect,
    GetWindowThreadProcessId, IsIconic, PostMessageW, SC_MINIMIZE, WM_CLOSE, WM_SYSCOMMAND,
};

use super::{Area, Process, text, visible};

/// A top-level window of another program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window(HWND);

/// The visible top-level windows of `process` that no other window owns,
/// in Windows' order, front first.
#[allow(unsafe_code)]
pub fn windows_of(Process(pid): Process) -> Vec<Window> {
    let mut all: Vec<HWND> = Vec::new();
    // SAFETY: the callback is a plain function; `all` outlives the call,
    // which reads it only through the pointer passed.
    unsafe { EnumWindows(Some(listed), &raw mut all as LPARAM) };
    all.into_iter()
        .filter(|&window| {
            let mut owner = 0u32;
            // SAFETY: any handle may be asked; `owner` is live.
            unsafe { GetWindowThreadProcessId(window, &mut owner) };
            // SAFETY: as above.
            let owned = unsafe { !GetWindow(window, GW_OWNER).is_null() };
            owner == pid && visible(window) && !owned
        })
        .map(Window)
        .collect()
}

#[allow(unsafe_code)]
unsafe extern "system" fn listed(window: HWND, all: LPARAM) -> i32 {
    // SAFETY: `all` is the vector `windows_of` passed, alive for the
    // enumeration.
    unsafe { (*(all as *mut Vec<HWND>)).push(window) };
    1
}

impl Window {
    /// Its title.
    pub fn title(self) -> String {
        text(self.0)
    }

    /// Where it is on the screen, while it is there.
    #[allow(unsafe_code)]
    pub fn area(self) -> Option<Area> {
        let mut rect = RECT::default();
        // SAFETY: any handle may be asked; `rect` is live.
        (unsafe { GetWindowRect(self.0, &mut rect) } != 0).then(|| Area::from(rect))
    }

    /// Where its client area, inside the frame, is on the screen.
    #[allow(unsafe_code)]
    pub fn client_area(self) -> Option<Area> {
        let mut rect = RECT::default();
        let mut corner = POINT::default();
        // SAFETY: any handle may be asked; `rect` and `corner` are live.
        let known = unsafe {
            GetClientRect(self.0, &mut rect) != 0 && ClientToScreen(self.0, &mut corner) != 0
        };
        known.then(|| Area {
            left: corner.x,
            top: corner.y,
            right: corner.x + rect.right,
            bottom: corner.y + rect.bottom,
        })
    }

    /// Whether it is the window in front.
    #[allow(unsafe_code)]
    pub fn in_front(self) -> bool {
        // SAFETY: plain call.
        unsafe { GetForegroundWindow() == self.0 }
    }

    /// Whether it is minimised.
    #[allow(unsafe_code)]
    pub fn minimised(self) -> bool {
        // SAFETY: any handle may be asked.
        unsafe { IsIconic(self.0) != 0 }
    }

    /// Asks it to minimise, as its minimise button does.
    ///
    /// # Errors
    /// The window is gone, or its queue is full.
    pub fn minimise(self) -> io::Result<()> {
        self.post(WM_SYSCOMMAND, SC_MINIMIZE as usize)
    }

    /// Asks it to close, as its close button does.
    ///
    /// # Errors
    /// The window is gone, or its queue is full.
    #[allow(unsafe_code)]
    pub fn close(self) -> io::Result<()> {
        self.post(WM_CLOSE, 0)
    }

    #[allow(unsafe_code)]
    fn post(self, message: u32, wparam: usize) -> io::Result<()> {
        // SAFETY: a posted message with no pointers.
        if unsafe { PostMessageW(self.0, message, wparam, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
