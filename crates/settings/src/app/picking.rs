//! The app's side of picking the fixed point: it opens the overlays on the
//! screens of the daemon's last view, asks the daemon where the clicker
//! clicks for the point picked, and fills the fields with it.
//!
//! Picking is offered only while the daemon is idle, and the overlays close
//! when it leaves idle or the screens change: a run's own click must never
//! land on them. The daemon answers a pick only while idle too.

use std::cell::RefCell;
use std::rc::Weak;

use auto_clicker_core::message::Unavailability;
use auto_clicker_core::window::{PICK_TIME, WindowView};
use slint::ComponentHandle;

use super::App;
use crate::pick::{self, Pick, PickEnd};
use crate::state::{self, Note};

impl App {
    /// Opens the overlays, when the user may ask and the screens are known.
    pub(super) fn pick(&mut self, app: Weak<RefCell<App>>) {
        let Some(view) = self.last.as_ref() else {
            return;
        };
        let Some(screens) = view.screens.as_ref() else {
            return;
        };
        if self.pick.is_some() || !self.state.may_ask() || !state::idle(view) {
            return;
        }
        let monitors = pick::monitors(self.window.window()).unwrap_or_default();
        let Some(targets) = pick::targets(monitors, screens) else {
            return self.note(&Note::Unavailable(Unavailability::ScreensChanged));
        };
        let ended = move |how| {
            if let Some(app) = app.upgrade() {
                app.borrow_mut().pick_ended(how);
            }
        };
        match Pick::open(targets, pick::place, PICK_TIME, ended) {
            Ok(opened) => {
                self.pick = Some(opened);
                self.note(&Note::Picking);
            }
            Err(why) => self.note(&Note::PickFailed(why)),
        }
        self.window.set_picking(self.pick.is_some());
    }

    /// The overlays ended: a picked point goes to the daemon to locate.
    fn pick_ended(&mut self, how: PickEnd) {
        self.close_pick();
        match how {
            PickEnd::At { point, on } => {
                if let Some(request) = self.state.locate(point, on) {
                    self.send(&request);
                }
            }
            PickEnd::Cancelled => self.note(&Note::PickCancelled),
            PickEnd::TimedOut => self.note(&Note::PickTimedOut),
        }
    }

    /// Closes the overlays, if they are up.
    pub(super) fn close_pick(&mut self) {
        self.pick = None;
        self.window.set_picking(false);
    }

    /// A new view closes the overlays when the program left idle or the
    /// screens changed under them, and says why.
    pub(super) fn pick_follows(&mut self, view: &WindowView) {
        if self.pick.is_none() {
            return;
        }
        let shown = self.last.as_ref().and_then(|last| last.screens.as_ref());
        let why = if !state::idle(view) {
            Note::Busy
        } else if view.screens.as_ref() != shown {
            Note::Unavailable(Unavailability::ScreensChanged)
        } else {
            return;
        };
        self.close_pick();
        self.note(&why);
    }

    /// Fills the fields with a picked point: the fixed point, at `x`, `y`.
    pub(super) fn fill_picked(&self, x: u16, y: u16) {
        self.window.set_position_fixed(true);
        self.window.set_position_x(i32::from(x));
        self.window.set_position_y(i32::from(y));
    }
}
