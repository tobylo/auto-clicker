//! Where the overlays go, through winit: the one place the window reaches
//! past Slint, since Slint cannot put a window on a chosen screen.
//!
//! An overlay is made full-screen on its monitor as it is created: the
//! backend's window-attributes hook, which Slint runs inside the overlay's
//! `new`, takes the monitor set just before. Asked at creation, the window
//! never shows at its own size first, and nothing has to wait for the
//! window to exist and then act on an overlay that may have closed since.
//!
//! winit reports a Wayland monitor's position as its logical position
//! times the monitor's whole-number scale (`wl_output.scale`, 2 for a
//! screen at 1.25); divided back, it is the compositor's logical corner.
//! The daemon moves the layout so its bounding box starts at (0, 0) (W01
//! brief, W3: niri and Hyprland allow negative positions), so every corner
//! here is moved by the same offset: the smallest left and top among the
//! monitors.

use std::cell::RefCell;

use i_slint_backend_winit::WinitWindowAccessor;
use i_slint_backend_winit::winit::monitor::MonitorHandle;
use i_slint_backend_winit::winit::window::{Fullscreen, WindowAttributes};

use super::Monitor;
use crate::ui::PickWindow;

/// A monitor an overlay goes on.
pub type Screen = MonitorHandle;

thread_local! {
    /// The monitor the window being created goes full-screen on.
    static CREATING_ON: RefCell<Option<MonitorHandle>> = const { RefCell::new(None) };
}

/// The backend's window-attributes hook: the window being created goes
/// full-screen on the monitor [`place`] set; any other window is as it was.
pub fn hook(attributes: WindowAttributes) -> WindowAttributes {
    match CREATING_ON.with(|on| on.borrow_mut().take()) {
        Some(monitor) => attributes.with_fullscreen(Some(Fullscreen::Borderless(Some(monitor)))),
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

/// The monitors `window` knows of, with their logical corners; `None` when
/// it is not a winit window. A monitor whose corner is not a whole logical
/// pixel is left out, which leaves its screen unmatched.
pub fn monitors(window: &slint::Window) -> Option<Vec<Monitor<MonitorHandle>>> {
    window.with_winit_window(|winit| from_origin(logical_corners(winit.available_monitors())))
}

/// The monitors with their logical corners, the ones not whole left out.
fn logical_corners(monitors: impl Iterator<Item = MonitorHandle>) -> Vec<Monitor<MonitorHandle>> {
    monitors
        .filter_map(|monitor| {
            let (left, top) = logical_corner(&monitor)?;
            Some(Monitor { left, top, monitor })
        })
        .collect()
}

fn logical_corner(monitor: &MonitorHandle) -> Option<(i64, i64)> {
    let scale = monitor.scale_factor();
    let position = monitor.position();
    Some((logical(position.x, scale)?, logical(position.y, scale)?))
}

/// `monitors` moved so the smallest left and top are 0, as the daemon
/// moves the layout.
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

/// Whether `window` was made full-screen and is not any more: the user
/// took an overlay out of full screen, so its corner is no longer the
/// screen's. Not for a window that is not a winit window.
pub fn left_full_screen(window: &slint::Window) -> bool {
    window
        .with_winit_window(|winit| winit.fullscreen().is_none())
        .unwrap_or(false)
}

/// The logical top-left corner of the monitor `window` is on now; `None`
/// when that is not known, as before the compositor said, or the window is
/// not a winit window.
pub fn corner(window: &slint::Window) -> Option<(i64, i64)> {
    window.with_winit_window(|winit| {
        let corner = logical_corner(&winit.current_monitor()?)?;
        let all = logical_corners(winit.available_monitors());
        Some(moved(
            corner,
            all.iter().map(|monitor| (monitor.left, monitor.top)),
        ))
    })?
}

/// `corner` moved as [`from_origin`] moves the monitors at `corners`.
fn moved((x, y): (i64, i64), corners: impl Iterator<Item = (i64, i64)>) -> (i64, i64) {
    let (left, top) = origin(corners);
    (x.saturating_sub(left), y.saturating_sub(top))
}

/// How many of the compositor's logical pixels one of Slint's is on
/// `window`: 1 unless `SLINT_SCALE_FACTOR` overrides the screen's scale,
/// or the window is not a winit window.
pub fn units(window: &slint::Window) -> f64 {
    let native = window.with_winit_window(|winit| winit.scale_factor());
    ratio(f64::from(window.scale_factor()), native)
}

/// Slint's scale over the screen's, or 1 when either is unknown or none.
fn ratio(slint: f64, native: Option<f64>) -> f64 {
    match native {
        Some(native) if native > 0.0 && slint > 0.0 => slint / native,
        _ => 1.0,
    }
}

/// `physical` divided by the whole-number `scale`, when it divides evenly.
fn logical(physical: i32, scale: f64) -> Option<i64> {
    let value = f64::from(physical) / scale;
    // Below 2^31 in size and whole: the cast is exact.
    (scale >= 1.0 && value.fract() == 0.0).then_some(value as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_corner_is_divided_back_by_the_whole_scale() {
        let cases = [
            ((5120, 2.0), Some(2560)),
            ((2048, 1.0), Some(2048)),
            ((0, 2.0), Some(0)),
            ((-3440, 1.0), Some(-3440)),
            // Not a whole logical pixel, or no scale.
            ((5121, 2.0), None),
            ((10, 0.0), None),
        ];
        for ((physical, scale), expected) in cases {
            assert_eq!(logical(physical, scale), expected, "{physical} {scale}");
        }
    }

    #[test]
    fn corners_are_moved_by_the_smallest_left_and_top() {
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
        // niri with one output at (-500, -300) and one right of it, lower.
        assert_eq!(
            corners(vec![monitor(-500, -300), monitor(1420, 100)]),
            [(0, 0), (1920, 400)]
        );
        // Plasma's layouts start at (0, 0) already.
        assert_eq!(
            corners(vec![monitor(2048, 0), monitor(0, 0)]),
            [(2048, 0), (0, 0)]
        );
        // The smallest left and top may come from different monitors.
        assert_eq!(
            corners(vec![monitor(-1920, 200), monitor(0, -1080)]),
            [(0, 1280), (1920, 0)]
        );
        assert!(corners(Vec::new()).is_empty());
        assert_eq!(origin(std::iter::empty()), (0, 0));
    }

    #[test]
    fn the_overlays_corner_is_moved_as_the_monitors_are() {
        let corners = [(-500, -300), (1420, 100)];
        let monitors = corners
            .iter()
            .map(|&(left, top)| Monitor {
                left,
                top,
                monitor: (),
            })
            .collect();
        let all: Vec<(i64, i64)> = from_origin(monitors)
            .iter()
            .map(|monitor| (monitor.left, monitor.top))
            .collect();
        for (corner, expected) in corners.into_iter().zip(all) {
            assert_eq!(moved(corner, corners.into_iter()), expected, "{corner:?}");
        }
        assert_eq!(moved((1420, 100), corners.into_iter()), (1920, 400));
    }

    #[test]
    fn monitors_at_negative_corners_match_the_layout_the_daemon_moved() {
        use auto_clicker_core::recording::{Layout, Output, Rect};

        let rect = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        // niri's outputs at (-500, -300) and (1420, 100), moved by the
        // daemon to start at (0, 0).
        let screens = Layout::new(vec![
            Output::new(rect(0, 0, 1920, 1080), 1000).unwrap(),
            Output::new(rect(1920, 400, 2560, 1440), 1000).unwrap(),
        ])
        .unwrap();
        let monitors = vec![
            Monitor {
                left: 1420,
                top: 100,
                monitor: "DP-1",
            },
            Monitor {
                left: -500,
                top: -300,
                monitor: "HDMI-A-1",
            },
        ];
        let matched: Vec<(&str, Rect)> = crate::pick::targets(from_origin(monitors), &screens)
            .unwrap()
            .into_iter()
            .map(|target| (target.monitor, target.on))
            .collect();
        assert_eq!(
            matched,
            [
                ("DP-1", rect(1920, 400, 2560, 1440)),
                ("HDMI-A-1", rect(0, 0, 1920, 1080))
            ]
        );
    }

    #[test]
    fn slints_units_are_its_scale_over_the_screens() {
        let cases = [
            ((1.25, Some(1.25)), 1.0),
            // SLINT_SCALE_FACTOR=2 on a screen at 1: a Slint pixel is two.
            ((2.0, Some(1.0)), 2.0),
            ((1.0, Some(2.0)), 0.5),
            ((1.25, None), 1.0),
            ((1.25, Some(0.0)), 1.0),
            ((0.0, Some(1.0)), 1.0),
        ];
        for ((slint, native), expected) in cases {
            assert_eq!(ratio(slint, native), expected, "{slint} {native:?}");
        }
    }

    #[test]
    fn only_the_window_made_next_is_placed() {
        let attributes = WindowAttributes::default();
        assert!(hook(attributes.clone()).fullscreen.is_none());
    }
}
