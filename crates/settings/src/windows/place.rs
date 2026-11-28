//! Where the overlays go on Windows, through winit, as on Linux: each is
//! made full-screen on its monitor as it is created, through the backend's
//! window-attributes hook, and kept above the taskbar.
//!
//! The window is per-monitor DPI aware (version 2), so winit reports each
//! monitor's corner in physical pixels on the virtual desktop, as the
//! daemon's layout counts them (WN09), moved so the smallest left and top
//! are 0. A click on an overlay is in Slint's logical pixels; times the
//! overlay's scale it is the physical pixel the layout counts.

use std::cell::RefCell;

use i_slint_backend_winit::WinitWindowAccessor;
use i_slint_backend_winit::winit::monitor::MonitorHandle;
use i_slint_backend_winit::winit::window::{Fullscreen, WindowAttributes, WindowLevel};

use crate::pick::Monitor;
use crate::ui::PickWindow;

/// A monitor an overlay goes on.
pub type Screen = MonitorHandle;

thread_local! {
    /// The monitor the window being created goes full-screen on.
    static CREATING_ON: RefCell<Option<MonitorHandle>> = const { RefCell::new(None) };
}

/// The backend's window-attributes hook: the window being created goes
/// full-screen on the monitor [`place`] set, above every other window; any
/// other window is as it was.
pub fn hook(attributes: WindowAttributes) -> WindowAttributes {
    match CREATING_ON.with(|on| on.borrow_mut().take()) {
        Some(monitor) => attributes
            .with_fullscreen(Some(Fullscreen::Borderless(Some(monitor))))
            .with_window_level(WindowLevel::AlwaysOnTop),
        None => attributes,
    }
}

/// An overlay, full-screen on `monitor`.
///
/// # Errors
/// The overlay could not be made.
pub fn place(monitor: MonitorHandle) -> Result<PickWindow, slint::PlatformError> {
    CREATING_ON.with(|on| *on.borrow_mut() = Some(monitor));
    let window = PickWindow::new();
    // Not left for a later window, should this one not have been made.
    CREATING_ON.with(|on| on.borrow_mut().take());
    window
}

/// The monitors `window` knows of, with their corners in the layout's
/// physical pixels; `None` when it is not a winit window.
pub fn monitors(window: &slint::Window) -> Option<Vec<Monitor<MonitorHandle>>> {
    window.with_winit_window(|winit| from_origin(corners(winit.available_monitors())))
}

fn corners(monitors: impl Iterator<Item = MonitorHandle>) -> Vec<Monitor<MonitorHandle>> {
    monitors
        .map(|monitor| {
            let (left, top) = corner_of(&monitor);
            Monitor { left, top, monitor }
        })
        .collect()
}

fn corner_of(monitor: &MonitorHandle) -> (i64, i64) {
    let position = monitor.position();
    (i64::from(position.x), i64::from(position.y))
}

/// `monitors` moved so the smallest left and top are 0, as the daemon's
/// layout counts from the virtual desktop's top-left corner.
fn from_origin<M>(monitors: Vec<Monitor<M>>) -> Vec<Monitor<M>> {
    let (left, top) = origin(monitors.iter().map(|monitor| (monitor.left, monitor.top)));
    monitors
        .into_iter()
        .map(|monitor| Monitor {
            left: monitor.left.saturating_sub(left),
            top: monitor.top.saturating_sub(top),
            ..monitor
        })
        .collect()
}

/// The smallest left and top among `corners`; (0, 0) when there are none.
fn origin(corners: impl Iterator<Item = (i64, i64)>) -> (i64, i64) {
    corners
        .reduce(|(left, top), (x, y)| (left.min(x), top.min(y)))
        .unwrap_or((0, 0))
}

/// Whether `window` was made full-screen and is not any more. Not for a
/// window that is not a winit window.
pub fn left_full_screen(window: &slint::Window) -> bool {
    window
        .with_winit_window(|winit| winit.fullscreen().is_none())
        .unwrap_or(false)
}

/// The corner of the monitor `window` is on now, in the layout's physical
/// pixels; `None` when that is not known, or the window is not a winit
/// window.
pub fn corner(window: &slint::Window) -> Option<(i64, i64)> {
    window.with_winit_window(|winit| {
        let (x, y) = corner_of(&winit.current_monitor()?);
        let (left, top) = origin(winit.available_monitors().map(|m| corner_of(&m)));
        Some((x.saturating_sub(left), y.saturating_sub(top)))
    })?
}

/// How many physical pixels one of Slint's logical pixels is on `window`:
/// its scale, or 1 when that is not known.
pub fn units(window: &slint::Window) -> f64 {
    let scale = f64::from(window.scale_factor());
    if scale > 0.0 { scale } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_count_from_the_virtual_desktops_top_left_corner() {
        let monitor = |left, top| Monitor {
            left,
            top,
            monitor: (),
        };
        let corners = |monitors: Vec<Monitor<()>>| -> Vec<(i64, i64)> {
            from_origin(monitors)
                .iter()
                .map(|monitor| (monitor.left, monitor.top))
                .collect()
        };
        // A second monitor left of the primary, whose corner is (0, 0).
        assert_eq!(
            corners(vec![monitor(0, 0), monitor(-2560, -360)]),
            [(2560, 360), (0, 0)]
        );
        assert_eq!(corners(vec![monitor(0, 0)]), [(0, 0)]);
        assert!(corners(Vec::new()).is_empty());
    }

    #[test]
    fn only_the_window_made_next_is_placed() {
        let attributes = WindowAttributes::default();
        let kept = hook(attributes.clone());
        assert!(kept.fullscreen.is_none());
        assert_eq!(kept.window_level, WindowLevel::Normal);
    }
}
