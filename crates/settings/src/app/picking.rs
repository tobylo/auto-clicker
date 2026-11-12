//! The app's side of picking the fixed point: it opens the overlays on the
//! screens of the daemon's last view, asks the daemon where the clicker
//! clicks for the point picked, and fills the fields with it.
//!
//! Picking is offered only while the daemon is idle, and the overlays close
//! when it leaves idle, a run begins or the screens change: a run's own
//! click must never land on them. The pick carries the counts of runs and
//! screen changes the view showed when it began, and the daemon refuses it
//! when either moved since, even if the window had not heard yet.

use std::cell::RefCell;
use std::rc::Weak;

use auto_clicker_core::message::Unavailability;
use auto_clicker_core::recording::Layout;
use auto_clicker_core::window::{PICK_TIME, Since, WindowView};
use slint::ComponentHandle;

use super::App;
use crate::pick::{self, Pick, PickEnd, Target};
use crate::state::{self, Note};

impl App {
    /// Opens the overlays, when the user may ask and the screens are known.
    pub(super) fn pick(&mut self, app: Weak<RefCell<App>>) {
        let Some((screens, since)) = self.pickable() else {
            return;
        };
        let monitors = pick::monitors(self.window.window()).unwrap_or_default();
        match pick::targets(monitors, &screens) {
            Some(targets) => self.open_pick(targets, since, app),
            None => self.note(&Note::Unavailable(Unavailability::ScreensChanged)),
        }
    }

    /// The screens to pick on and what the pick is checked against, when a
    /// pick may begin.
    fn pickable(&self) -> Option<(Layout, Since)> {
        pickable(
            self.last.as_ref(),
            self.state.may_ask(),
            self.pick.is_some(),
        )
    }

    fn open_pick(
        &mut self,
        targets: Vec<Target<pick::Screen>>,
        since: Since,
        app: Weak<RefCell<App>>,
    ) {
        let ended = move |how| {
            if let Some(app) = app.upgrade() {
                app.borrow_mut().pick_ended(how);
            }
        };
        match Pick::open(targets, pick::place, PICK_TIME, ended) {
            Ok(opened) => {
                self.pick = Some((opened, since));
                self.note(&Note::Picking);
            }
            Err(why) => self.note(&Note::PickFailed(why)),
        }
        self.window.set_picking(self.pick.is_some());
    }

    /// The overlays ended: a picked point goes to the daemon to locate.
    fn pick_ended(&mut self, how: PickEnd) {
        let since = self.close_pick();
        match (how, since) {
            (PickEnd::At { at, on }, Some(since)) => {
                if let Some(request) = self.state.locate(at, on, since) {
                    self.send(&request);
                }
            }
            (PickEnd::Cancelled, _) => self.note(&Note::PickCancelled),
            (PickEnd::TimedOut, _) => self.note(&Note::PickTimedOut),
            // An end told after the pick was closed: nothing is up to end.
            (PickEnd::At { .. }, None) => {}
        }
    }

    /// Closes the overlays, if they are up; what the pick was checked
    /// against, if one was.
    pub(super) fn close_pick(&mut self) -> Option<Since> {
        self.window.set_picking(false);
        self.pick.take().map(|(_, since)| since)
    }

    /// A new view closes the overlays when the program left idle or the
    /// screens changed under them, and says why.
    pub(super) fn pick_follows(&mut self, view: &WindowView) {
        let Some((_, since)) = &self.pick else {
            return;
        };
        if let Some(why) = ends_pick(*since, view) {
            self.close_pick();
            self.note(&why);
        }
    }

    /// Fills the fields with a picked point: the fixed point, at `x`, `y`.
    pub(super) fn fill_picked(&self, x: u16, y: u16) {
        self.window.set_position_fixed(true);
        self.window.set_position_x(i32::from(x));
        self.window.set_position_y(i32::from(y));
    }
}

/// The screens to pick on and the view's counts, when a pick may begin:
/// the last view shows the screens, the program is idle, the user may ask
/// and no pick is up.
fn pickable(last: Option<&WindowView>, may_ask: bool, picking: bool) -> Option<(Layout, Since)> {
    let view = last?;
    let free = may_ask && !picking;
    let screens = view.screens.clone().filter(|_| free && state::idle(view))?;
    Some((screens, view.since))
}

/// Why `view` ends a pick begun on the counts `since`: the program left
/// idle or a run began, or the screens changed.
fn ends_pick(since: Since, view: &WindowView) -> Option<Note> {
    if !state::idle(view) || view.since.runs != since.runs {
        Some(Note::Busy)
    } else if view.since.layouts != since.layouts {
        Some(Note::Unavailable(Unavailability::ScreensChanged))
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
