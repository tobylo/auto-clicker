//! A point the user picked on the screens in the settings window: where the
//! clicker clicks for it, by decision 0001's mapping, on the screens as
//! KWin has them now.
//!
//! The window picks on the layout its view showed; the screens may have
//! changed since. A point is located only when the screens have not
//! changed since the pick began, the output it was picked on is there in
//! the same place and size, and the point lies on it. It is answered only
//! while idle and when no run began since the pick did: a run's own click,
//! even one that ended before the window's request came, must never be
//! taken for a pick. The window offers picking only while idle too.
//!
//! A point saved from a pick keeps the screens it was picked on. Clicking
//! at it starts only on those screens, and stops when they change: on other
//! screens the same numbers are another place.

use super::{Controller, Job, Platform, State};
use crate::clicker::Position;
use crate::message::{Hindrance, Notice, PointScreens, Start, Stop, Unavailability};
use crate::recording::{Layout, Rect};
use crate::window::{Located, Outcome, Since, Spot};

impl<P: Platform> Controller<P> {
    /// Why clicking with the config's settings would land elsewhere than
    /// the point was picked: only a click mode moves to it, and only a
    /// picked point knows its screens.
    fn point_screens(&self) -> Option<PointScreens> {
        let clicker = &self.config.clicker;
        let moves =
            clicker.pressed_key().is_none() && matches!(clicker.position, Position::Fixed(_));
        let picked = self.config.fixed_screens.as_ref().filter(|_| moves)?;
        match self.platform.layout() {
            Some(now) if now.same_screens(picked) => None,
            Some(_) => Some(PointScreens::Changed),
            None => Some(PointScreens::Unknown),
        }
    }

    /// Whether clicking may start on the screens now; says why not when it
    /// may not.
    pub(super) fn screens_allow(&mut self) -> bool {
        let Some(why) = self.point_screens() else {
            return true;
        };
        self.notice = Some(Notice::Refused {
            start: Start::Clicking,
            why: Hindrance::Screens(why),
        });
        false
    }

    /// The screens changed: clicking at a picked point, or about to, stops
    /// when they are no longer the ones it was picked on.
    pub(super) fn screens_changed_for_clicking(&mut self) {
        let clicking = match &self.state {
            State::Running(running) => running.playing.is_none(),
            State::Countdown(_, job) | State::Release(_, job) | State::Opening(job) => {
                matches!(job, Job::Click)
            }
            State::Idle | State::RecordCountdown(_) | State::Recording(_) => false,
        };
        if let Some(why) = self.point_screens().filter(|_| clicking) {
            self.stop(Some(Stop::Screens(why)));
        }
    }

    /// Where the clicker clicks for `at`, picked on the output at `on` in a
    /// pick begun when the view showed `since`.
    pub(super) fn locate(&self, at: Spot, on: Rect, since: Since) -> Outcome {
        if !self.settled() || since.runs != self.since.runs {
            return Outcome::Busy;
        }
        match self.platform.layout() {
            None => Outcome::Unavailable(Unavailability::ScreensUnknown),
            Some(_) if since.layouts != self.since.layouts => {
                Outcome::Unavailable(Unavailability::ScreensChanged)
            }
            Some(screens) => located(screens, at, on),
        }
    }
}

/// `at` on `screens`, when the output at `on` is still one of them and
/// holds it.
fn located(screens: Layout, at: Spot, on: Rect) -> Outcome {
    let still_there = screens.outputs().iter().any(|output| output.rect() == on);
    if !still_there || !on.holds(at.x(), at.y()) {
        return Outcome::Unavailable(Unavailability::ScreensChanged);
    }
    let absolute = screens.absolute_at(at.x(), at.y());
    Outcome::Located(Located {
        x: absolute.x,
        y: absolute.y,
        screens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::Output;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// The laptop's panel at 1.25 with DP-1 at 1.5 to its right: KWin's
    /// numbers from decision 0001, a 4341 x 1280 layout.
    fn screens() -> Layout {
        Layout::new(vec![
            Output::new(rect(0, 0, 2048, 1280), 1250).unwrap(),
            Output::new(rect(2048, 0, 2293, 960), 1500).unwrap(),
        ])
        .unwrap()
    }

    fn at(x: u16, y: u16) -> Outcome {
        Outcome::Located(Located {
            x,
            y,
            screens: screens(),
        })
    }

    fn spot(x: f64, y: f64) -> Spot {
        Spot::new(x, y).unwrap()
    }

    #[test]
    fn a_point_on_either_screen_maps_by_the_formula() {
        let panel = rect(0, 0, 2048, 1280);
        let dp1 = rect(2048, 0, 2293, 960);
        let cases = [
            // round(1024 * 65536 / 4341) = 15459; round(640 * 65536 / 1280) = 32768.
            (spot(1024.0, 640.0), panel, at(15459, 32768)),
            // round(1000.36 * 65536 / 4341) = 15102; round(640.55 * 65536 / 1280) = 32796.
            (spot(1000.36, 640.55), panel, at(15102, 32796)),
            // DP-1's first pixel: round(2048 * 65536 / 4341) = 30919.
            (spot(2048.0, 0.0), dp1, at(30919, 0)),
            // DP-1's last fraction of a pixel, past KWin's whole-pixel width
            // at scale 1.5: round(4340.9 * 65536 / 4341) = 65534,
            // round(959.9 * 65536 / 1280) = 49147.
            (spot(4340.9, 959.9), dp1, at(65534, 49147)),
        ];
        for (point, on, expected) in cases {
            assert_eq!(located(screens(), point, on), expected, "{point:?}");
        }
    }

    #[test]
    fn a_point_off_its_screen_or_a_screen_gone_is_refused() {
        let changed = Outcome::Unavailable(Unavailability::ScreensChanged);
        let cases = [
            // Below DP-1, inside the bounding box: on no screen.
            (spot(3000.0, 960.0), rect(2048, 0, 2293, 960)),
            // On the panel, but said to be picked on DP-1.
            (spot(10.0, 10.0), rect(2048, 0, 2293, 960)),
            // DP-1 as KScreen rounds it, 2294 wide: not KWin's screen.
            (spot(2100.0, 10.0), rect(2048, 0, 2294, 960)),
            // A screen that is not there.
            (spot(10.0, 10.0), rect(0, 0, 1920, 1080)),
        ];
        for (point, on) in cases {
            assert_eq!(located(screens(), point, on), changed, "{point:?} {on:?}");
        }
    }
}
