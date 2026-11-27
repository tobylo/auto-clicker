//! The screen as a device test sees and changes it: where the pointer is,
//! the primary screen's size, and its display mode, changed for a test and
//! put back however the test ends.

use std::io;
use std::ptr::null;

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsW, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_PELSHEIGHT, DM_PELSWIDTH,
    EnumDisplaySettingsW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
};

use super::Point;

/// Where the pointer is, in physical pixels in a per-monitor aware process.
///
/// # Errors
/// Windows could not say (no desktop, as over SSH).
#[allow(unsafe_code)]
pub fn pointer() -> io::Result<Point> {
    let mut at = POINT::default();
    // SAFETY: `at` is a live, writable `POINT`.
    if unsafe { GetCursorPos(&mut at) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Point { x: at.x, y: at.y })
}

/// The primary screen's width and height.
#[allow(unsafe_code)]
pub fn screen_size() -> (i32, i32) {
    // SAFETY: the metrics' indices alone.
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

/// The primary screen in another display mode, until dropped: then the
/// mode in the registry, the one it had, comes back.
#[derive(Debug)]
pub struct DisplayMode {
    _private: (),
}

/// The sizes the primary screen's adapter offers, each once.
#[allow(unsafe_code)]
pub fn display_sizes() -> Vec<(u32, u32)> {
    let mut sizes = Vec::new();
    for index in 0.. {
        let mut mode = DEVMODEW {
            dmSize: u16::try_from(size_of::<DEVMODEW>()).unwrap_or(u16::MAX),
            ..DEVMODEW::default()
        };
        // SAFETY: the primary screen (no name), mode `index`, and a
        // `DEVMODEW` of its own size to fill.
        if unsafe { EnumDisplaySettingsW(null(), index, &mut mode) } == 0 {
            break;
        }
        let size = (mode.dmPelsWidth, mode.dmPelsHeight);
        if !sizes.contains(&size) {
            sizes.push(size);
        }
    }
    sizes
}

impl DisplayMode {
    /// Changes the primary screen to `width` by `height` for now, without
    /// saving it.
    ///
    /// # Errors
    /// The mode is not one the screen has, or Windows refused it.
    #[allow(unsafe_code)]
    pub fn set(width: u32, height: u32) -> io::Result<DisplayMode> {
        let mode = DEVMODEW {
            dmSize: u16::try_from(size_of::<DEVMODEW>()).unwrap_or(u16::MAX),
            dmFields: DM_PELSWIDTH | DM_PELSHEIGHT,
            dmPelsWidth: width,
            dmPelsHeight: height,
            ..DEVMODEW::default()
        };
        // SAFETY: a fully set `DEVMODEW` of its own size; flags 0 change the
        // mode for now and save nothing.
        let changed = unsafe { ChangeDisplaySettingsW(&mode, 0) };
        if changed != DISP_CHANGE_SUCCESSFUL {
            return Err(io::Error::other(format!(
                "the display mode {width}x{height} was refused: {changed}"
            )));
        }
        Ok(DisplayMode { _private: () })
    }
}

impl Drop for DisplayMode {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: no mode, flags 0: the registry's mode comes back.
        unsafe { ChangeDisplaySettingsW(null(), 0) };
    }
}
