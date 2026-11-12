//! Picking the clicker's fixed point on the screens: a full-screen overlay
//! on every screen of the view's layout. The next left click picks; Esc,
//! closing an overlay and the time limit ([`PICK_TIME`]) cancel. However it ends, every
//! overlay closes at once, and so they do when the pick is dropped: the
//! settings window closing, the daemon going or the program leaving idle.
//!
//! An overlay's click is in logical pixels from its screen's corner, and a
//! full-screen window's corner is its screen's: the screen's place in the
//! layout plus the click is the point on the layout (decision 0001).

mod place;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use auto_clicker_core::recording::{Layout, Point, Rect};
#[cfg(doc)]
use auto_clicker_core::window::PICK_TIME;
use slint::{CloseRequestResponse, ComponentHandle, JoinHandle, Timer, TimerMode};

use crate::ui::PickWindow;

pub use place::{monitors, place};

/// How a pick ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickEnd {
    /// The user clicked `point`, on the screen at `on`.
    At {
        point: Point,
        on: Rect,
    },
    Cancelled,
    TimedOut,
}

/// A monitor as the window sees it: its top-left corner in logical pixels,
/// and the monitor itself, for placing an overlay on it.
pub struct Monitor<M> {
    pub left: i64,
    pub top: i64,
    pub monitor: M,
}

/// A screen of the layout, and the monitor that shows it.
pub struct Target<M> {
    pub on: Rect,
    pub monitor: M,
}

/// Each monitor with the screen of `screens` it shows, matched by their
/// top-left corners. `None` when a monitor is on no screen of the layout or
/// a screen has no monitor: the window and KWin see different screens, as
/// while one is plugged in.
pub fn targets<M>(monitors: Vec<Monitor<M>>, screens: &Layout) -> Option<Vec<Target<M>>> {
    let rects: Vec<Rect> = screens
        .outputs()
        .iter()
        .map(|output| output.rect())
        .collect();
    let corner =
        |rect: &Rect, left: i64, top: i64| i64::from(rect.x) == left && i64::from(rect.y) == top;
    let mut targets = Vec::new();
    for Monitor { left, top, monitor } in monitors {
        let on = *rects.iter().find(|rect| corner(rect, left, top))?;
        targets.push(Target { on, monitor });
    }
    let covered = rects.iter().all(|rect| {
        targets
            .iter()
            .any(|target| corner(rect, target.on.x.into(), target.on.y.into()))
    });
    covered.then_some(targets)
}

/// The point on the layout for a click at (`x`, `y`) on the screen at
/// `on`, rounded to a whole pixel on that screen. A click on the last
/// fraction of a pixel stays on its screen: at a fractional scale the
/// overlay can reach past KWin's whole-pixel size.
pub fn point_on(on: Rect, x: f32, y: f32) -> Point {
    Point {
        x: on.x.saturating_add(axis(x, on.width)),
        y: on.y.saturating_add(axis(y, on.height)),
    }
}

/// `value` rounded, from 0 to the last pixel of `size`.
fn axis(value: f32, size: u32) -> u32 {
    let last = size.saturating_sub(1);
    let rounded = f64::from(value).round();
    if rounded.is_nan() || rounded <= 0.0 {
        0
    } else if rounded >= f64::from(last) {
        last
    } else {
        // A whole number from 1 to `last - 1`: the cast is exact.
        rounded as u32
    }
}

/// What tells the app how a pick ended: the first end only.
type Ending = Rc<RefCell<Option<Box<dyn FnOnce(PickEnd)>>>>;

fn end(ending: &Ending, how: PickEnd) {
    let taken = ending.borrow_mut().take();
    if let Some(told) = taken {
        told(how);
    }
}

/// The overlays of a pick that is up; dropping it closes them all.
pub struct Pick {
    windows: Vec<PickWindow>,
    /// The tasks placing each overlay on its screen.
    placing: Vec<JoinHandle<()>>,
    _timeout: Timer,
}

/// Puts an overlay on its monitor; the task doing it, if it takes one.
pub type Placer<M> = fn(&PickWindow, M) -> Result<Option<JoinHandle<()>>, String>;

impl Pick {
    /// Opens an overlay on each of `targets`, placed by `place`, for at
    /// most `time` ([`PICK_TIME`]). `ended` hears how the pick ended, once;
    /// it does not hear a pick dropped before it ended.
    ///
    /// # Errors
    /// An overlay could not be made, shown or placed; the ones opened so
    /// far are closed.
    pub fn open<M>(
        targets: Vec<Target<M>>,
        place: Placer<M>,
        time: Duration,
        ended: impl FnOnce(PickEnd) + 'static,
    ) -> Result<Pick, String> {
        let ending: Ending = Rc::new(RefCell::new(Some(Box::new(ended))));
        let timeout = Timer::default();
        let timed_out = Rc::clone(&ending);
        timeout.start(TimerMode::SingleShot, time, move || {
            end(&timed_out, PickEnd::TimedOut);
        });
        let mut pick = Pick {
            windows: Vec::new(),
            placing: Vec::new(),
            _timeout: timeout,
        };
        for Target { on, monitor } in targets {
            let window = overlay(on, &ending).map_err(|err| err.to_string())?;
            // Kept before it is shown, so a failure from here on closes it.
            pick.windows.push(window.clone_strong());
            window.show().map_err(|err| err.to_string())?;
            pick.placing.extend(place(&window, monitor)?);
        }
        Ok(pick)
    }
}

/// An overlay for the screen at `on`, telling `ending` how it ended.
fn overlay(on: Rect, ending: &Ending) -> Result<PickWindow, slint::PlatformError> {
    let window = PickWindow::new()?;
    let clicked = Rc::clone(ending);
    window.on_picked(move |x, y| {
        let point = point_on(on, x, y);
        end(&clicked, PickEnd::At { point, on });
    });
    let escaped = Rc::clone(ending);
    window.on_cancelled(move || end(&escaped, PickEnd::Cancelled));
    let closed = Rc::clone(ending);
    window.window().on_close_requested(move || {
        end(&closed, PickEnd::Cancelled);
        CloseRequestResponse::HideWindow
    });
    Ok(window)
}

impl Drop for Pick {
    fn drop(&mut self) {
        for window in &self.windows {
            let _ = window.hide();
        }
        for task in self.placing.drain(..) {
            task.abort();
        }
        // An overlay may be closing from its own callback: its handle goes
        // once that callback has returned.
        let windows = std::mem::take(&mut self.windows);
        Timer::single_shot(Duration::ZERO, move || drop(windows));
    }
}

#[cfg(test)]
pub(crate) mod tests;
