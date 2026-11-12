//! Where the overlays go, through winit: the one place the window reaches
//! past Slint, since Slint cannot put a window on a chosen screen.
//!
//! winit reports a Wayland monitor's position as its logical position
//! times the monitor's whole-number scale (`wl_output.scale`, 2 for a
//! screen at 1.25); divided back, it is KWin's logical corner.

use i_slint_backend_winit::WinitWindowAccessor;
use i_slint_backend_winit::winit::monitor::MonitorHandle;
use i_slint_backend_winit::winit::window::Fullscreen;
use slint::{ComponentHandle, JoinHandle};

use super::Monitor;
use crate::ui::PickWindow;

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

/// `physical` divided by the whole-number `scale`, when it divides evenly.
fn logical(physical: i32, scale: f64) -> Option<i64> {
    let value = f64::from(physical) / scale;
    // Below 2^31 in size and whole: the cast is exact.
    (scale >= 1.0 && value.fract() == 0.0).then_some(value as i64)
}

/// Makes `window` full-screen on `monitor` once its winit window exists,
/// which is after the event loop has run.
///
/// # Errors
/// No task can be started: there is no event loop.
pub fn place(
    window: &PickWindow,
    monitor: MonitorHandle,
) -> Result<Option<JoinHandle<()>>, String> {
    let window = window.clone_strong();
    let task = slint::spawn_local(async move {
        // Borrowed for this call only: a winit window held on to would
        // stay on screen after the overlay closes.
        if let Ok(winit) = window.window().winit_window().await {
            winit.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor))));
        }
    });
    task.map(Some).map_err(|err| err.to_string())
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
}
