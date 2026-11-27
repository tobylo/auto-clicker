//! The screens (M5 brief decision 10): the monitors in physical pixels, as
//! a per-monitor DPI aware process reads them (`dpi.rs`), and each
//! monitor's DPI scale.
//!
//! Windows counts the virtual desktop from the primary monitor's corner, so
//! a monitor left of or above it starts at a negative point. Core's layout
//! starts at (0, 0), so the monitors are counted from the virtual desktop's
//! top-left corner instead: the corner output's absolute moves are counted
//! from too (`MOUSEEVENTF_VIRTUALDESK`). A monitor's scale is its effective
//! DPI over 96, in thousandths (120 DPI is 1250); nothing compares it, since
//! a physical pixel is the same pixel at any scale.
//!
//! The message thread reads the screens when it starts and again on
//! `WM_DISPLAYCHANGE` and `WM_DPICHANGED`, and keeps the last reading for
//! the runtime, which reads it without waiting.

use std::ptr::{null, null_mut};
use std::sync::PoisonError;

use auto_clicker_core::message::NoScreens;
use auto_clicker_core::recording::{Layout, Output, Rect};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{WM_DISPLAYCHANGE, WM_DPICHANGED};
use windows_sys::core::BOOL;

use crate::message_thread::{Remote, WindowEvent, emit};

/// The DPI Windows calls 100 %.
const BASE_DPI: u32 = 96;

/// One monitor as Windows reports it: its rectangle on the virtual
/// desktop, in physical pixels, and its effective DPI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Monitor {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
    pub(crate) dpi: u32,
}

impl Remote {
    /// The screens as the thread last read them, without waiting: since
    /// its window was made, the latest reading is there before the change
    /// is told ([`WindowEvent::ScreensChanged`]).
    pub fn screens(&self) -> Result<Layout, NoScreens> {
        self.screens_slot()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Keeps `read` as the screens; whether they differ from those kept.
    pub(crate) fn keep_screens(&self, read: Result<Layout, NoScreens>) -> bool {
        let mut kept = self
            .screens_slot()
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let changed = *kept != read;
        *kept = read;
        changed
    }
}

/// What the window answers to a display change, on the message thread:
/// the screens are read again, and when they changed the handler hears
/// it. `None` for any other message, and for a change that left them as
/// they were, which is then only a wake like any other.
pub(crate) fn answer(message: u32) -> Option<LRESULT> {
    if message != WM_DISPLAYCHANGE && message != WM_DPICHANGED {
        return None;
    }
    let mut changed = false;
    Remote::on_thread(|remote| changed = remote.keep_screens(read()));
    changed.then(|| {
        emit(WindowEvent::ScreensChanged);
        0
    })
}

/// The screens now; [`NoScreens::NotWhole`] when Windows did not describe
/// every monitor, or described more than a layout holds.
pub(crate) fn read() -> Result<Layout, NoScreens> {
    layout_of(&monitors().ok_or(NoScreens::NotWhole)?)
}

/// The layout of `monitors`, counted from their bounding box's top-left
/// corner, each at its DPI's scale. [`NoScreens::NotWhole`] for none, more
/// than a layout holds, or one with no area, no DPI or a side past what a
/// layout counts.
pub(crate) fn layout_of(monitors: &[Monitor]) -> Result<Layout, NoScreens> {
    let left = monitors.iter().map(|monitor| monitor.left).min();
    let top = monitors.iter().map(|monitor| monitor.top).min();
    let (Some(left), Some(top)) = (left, top) else {
        return Err(NoScreens::NotWhole);
    };
    let outputs = monitors
        .iter()
        .map(|monitor| output(monitor, left, top))
        .collect::<Option<Vec<_>>>()
        .ok_or(NoScreens::NotWhole)?;
    Layout::new(outputs).map_err(|_| NoScreens::NotWhole)
}

/// `monitor` as an output, counted from (`left`, `top`).
fn output(monitor: &Monitor, left: i32, top: i32) -> Option<Output> {
    let from = |value: i32, origin: i32| u32::try_from(i64::from(value) - i64::from(origin)).ok();
    let rect = Rect {
        x: from(monitor.left, left)?,
        y: from(monitor.top, top)?,
        width: from(monitor.right, monitor.left)?,
        height: from(monitor.bottom, monitor.top)?,
    };
    Output::new(rect, scale_milli(monitor.dpi)?).ok()
}

/// The scale of `dpi`, in thousandths, to the nearest; `None` for none.
fn scale_milli(dpi: u32) -> Option<u32> {
    let scale = u64::from(dpi)
        .checked_mul(1000)?
        .checked_add(u64::from(BASE_DPI / 2))?
        / u64::from(BASE_DPI);
    u32::try_from(scale).ok().filter(|&scale| scale > 0)
}

/// Every monitor Windows enumerates; `None` when the enumeration, or a
/// monitor's description, failed.
#[allow(unsafe_code)]
fn monitors() -> Option<Vec<Monitor>> {
    let mut found = Found {
        monitors: Vec::new(),
        failed: false,
    };
    // SAFETY: no device context and no clip, so every monitor of the
    // virtual desktop; `found` outlives the call, which hands it back to
    // `each` only during it.
    let enumerated =
        unsafe { EnumDisplayMonitors(null_mut(), null(), Some(each), (&raw mut found) as LPARAM) };
    (enumerated != 0 && !found.failed).then_some(found.monitors)
}

/// What the enumeration found.
struct Found {
    monitors: Vec<Monitor>,
    failed: bool,
}

/// The enumeration's callback, for one monitor.
#[allow(unsafe_code)]
unsafe extern "system" fn each(
    handle: HMONITOR,
    _context: HDC,
    _area: *mut RECT,
    found: LPARAM,
) -> BOOL {
    // SAFETY: `found` is the `Found` `monitors` passed, alive and not
    // otherwise borrowed during the enumeration.
    let found = unsafe { &mut *(found as *mut Found) };
    match describe(handle) {
        Some(monitor) => found.monitors.push(monitor),
        None => found.failed = true,
    }
    // Every monitor is described, or the failure counts the whole reading.
    i32::from(!found.failed)
}

/// The monitor `handle` names: its rectangle and its effective DPI.
#[allow(unsafe_code)]
fn describe(handle: HMONITOR) -> Option<Monitor> {
    let mut info = MONITORINFO {
        cbSize: u32::try_from(size_of::<MONITORINFO>()).ok()?,
        ..MONITORINFO::default()
    };
    // SAFETY: a monitor handle the enumeration gave, and `info` with its
    // size set, which Windows fills.
    if unsafe { GetMonitorInfoW(handle, &mut info) } == 0 {
        return None;
    }
    let (mut dpi, mut vertical) = (0, 0);
    // SAFETY: the same handle; two live integers to fill.
    if unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi, &mut vertical) } != 0 {
        return None;
    }
    let RECT {
        left,
        top,
        right,
        bottom,
    } = info.rcMonitor;
    Some(Monitor {
        left,
        top,
        right,
        bottom,
        dpi,
    })
}

#[cfg(test)]
mod tests;
