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
//! screen at 1.25); divided back, it is KWin's logical corner.

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
    window.with_winit_window(|winit| {
        winit
            .available_monitors()
            .filter_map(|monitor| {
                let scale = monitor.scale_factor();
                let position = monitor.position();
                let left = logical(position.x, scale)?;
                let top = logical(position.y, scale)?;
                Some(Monitor { left, top, monitor })
            })
            .collect()
    })
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
