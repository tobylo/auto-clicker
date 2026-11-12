//! Picking the clicker's fixed point on the screens: a full-screen overlay
//! on every screen of the view's layout. The next left click picks; Esc,
//! closing an overlay and the time limit ([`PICK_TIME`]) cancel. However it ends, every
//! overlay closes at once, and so they do when the pick is dropped: the
//! settings window closing, the daemon going or the program leaving idle.
//!
//! An overlay's click is in logical pixels from its screen's corner, and a
//! full-screen window's corner is its screen's: the screen's place in the
//! layout plus the click is the point on the layout (decision 0001), to a
//! fraction of a pixel.

mod place;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use auto_clicker_core::recording::{Layout, Rect};
#[cfg(doc)]
use auto_clicker_core::window::PICK_TIME;
use auto_clicker_core::window::Spot;
use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};

use crate::ui::PickWindow;

pub use place::{Screen, hook, monitors, place, units};

/// How a pick ended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickEnd {
    /// The user clicked `at`, on the screen at `on`.
    At {
        at: Spot,
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
/// top-left corners, one per corner: winit lists a Wayland output more than
/// once. Screens of different sizes on one corner (mirrored) take the one
/// that contains the others, so a click anywhere on any of their monitors
/// is on it; when none contains the others, picking is refused. `None` when a monitor is on no screen of the layout or a screen
/// has no monitor: the window and KWin see different screens, as while one
/// is plugged in.
pub fn targets<M>(monitors: Vec<Monitor<M>>, screens: &Layout) -> Option<Vec<Target<M>>> {
    let rects: Vec<Rect> = screens
        .outputs()
        .iter()
        .map(|output| output.rect())
        .collect();
    let rects = covering(&rects)?;
    let corner =
        |rect: &Rect, left: i64, top: i64| i64::from(rect.x) == left && i64::from(rect.y) == top;
    let mut targets = Vec::new();
    for Monitor { left, top, monitor } in monitors {
        let on = *rects.iter().find(|rect| corner(rect, left, top))?;
        if !targets.iter().any(|target: &Target<M>| target.on == on) {
            targets.push(Target { on, monitor });
        }
    }
    let covered = rects.iter().all(|rect| {
        targets
            .iter()
            .any(|target| corner(rect, target.on.x.into(), target.on.y.into()))
    });
    covered.then_some(targets)
}

/// One rectangle per top-left corner: the one that contains every other
/// on its corner; `None` when one corner has no such rectangle.
fn covering(rects: &[Rect]) -> Option<Vec<Rect>> {
    let same_corner = |a: &Rect, b: &Rect| (a.x, a.y) == (b.x, b.y);
    let contains =
        |outer: &Rect, inner: &Rect| outer.width >= inner.width && outer.height >= inner.height;
    let mut chosen: Vec<Rect> = Vec::new();
    for rect in rects {
        if chosen.iter().any(|kept| same_corner(kept, rect)) {
            continue;
        }
        let group: Vec<&Rect> = rects
            .iter()
            .filter(|other| same_corner(other, rect))
            .collect();
        let outer = group
            .iter()
            .find(|outer| group.iter().all(|inner| contains(outer, inner)))?;
        chosen.push(**outer);
    }
    Some(chosen)
}

/// The position on the layout for a click at (`x`, `y`) on the screen at
/// `on`, kept on that screen: at a fractional scale the overlay can reach
/// past KWin's whole-pixel size. `None` for a position off the screens,
/// which no click on an overlay gives.
pub fn spot_on(on: Rect, x: f64, y: f64) -> Option<Spot> {
    let within = |start: u32, size: u32, value: f64| {
        // Short of the far edge, which is the next screen's; NaN gives 0.
        let last = (f64::from(size) - 1.0 / 1024.0).max(0.0);
        f64::from(start) + value.clamp(0.0, last).max(0.0)
    };
    Spot::new(within(on.x, on.width, x), within(on.y, on.height, y))
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
    _timeout: Timer,
}

/// Makes an overlay on a monitor.
pub type Placer<M> = fn(M) -> Result<PickWindow, slint::PlatformError>;

impl Pick {
    /// Opens an overlay on each of `targets`, made by `place`, for at
    /// most `time` ([`PICK_TIME`]). `ended` hears how the pick ended, once;
    /// it does not hear a pick dropped before it ended.
    ///
    /// # Errors
    /// An overlay could not be made or shown; the ones opened so far are
    /// closed.
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
            _timeout: timeout,
        };
        for Target { on, monitor } in targets {
            let window = place(monitor).map_err(|err| err.to_string())?;
            told_by(&window, on, &ending);
            // Kept before it is shown, so a failure from here on closes it.
            pick.windows.push(window.clone_strong());
            window.show().map_err(|err| err.to_string())?;
        }
        Ok(pick)
    }
}

/// Makes `window`, the overlay for the screen at `on`, tell `ending` how
/// the pick ended.
fn told_by(window: &PickWindow, on: Rect, ending: &Ending) {
    let clicked = Rc::clone(ending);
    let overlay = window.as_weak();
    window.on_picked(move |x, y| {
        // Slint's own scale (`SLINT_SCALE_FACTOR`) may differ from the
        // screen's: the click in the compositor's logical pixels.
        let units = overlay.upgrade().map_or(1.0, |w| units(w.window()));
        if let Some(at) = spot_on(on, f64::from(x) * units, f64::from(y) * units) {
            end(&clicked, PickEnd::At { at, on });
        }
    });
    let escaped = Rc::clone(ending);
    window.on_cancelled(move || end(&escaped, PickEnd::Cancelled));
    let closed = Rc::clone(ending);
    window.window().on_close_requested(move || {
        end(&closed, PickEnd::Cancelled);
        CloseRequestResponse::HideWindow
    });
}

impl Drop for Pick {
    fn drop(&mut self) {
        for window in &self.windows {
            let _ = window.hide();
        }
        // An overlay may be closing from its own callback: its handle goes
        // once that callback has returned.
        let windows = std::mem::take(&mut self.windows);
        Timer::single_shot(Duration::ZERO, move || drop(windows));
    }
}

#[cfg(test)]
pub(crate) mod tests;
