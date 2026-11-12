//! A point the user picked on the screens in the settings window: where the
//! clicker clicks for it, by decision 0001's mapping, on the screens as
//! KWin has them now.
//!
//! The window picks on the layout its view showed; the screens may have
//! changed since. A point is located only when the output it was picked on
//! is still there, in the same place and size, and the point lies on it.
//! It is answered only while idle: a run's own click must never be taken
//! for a pick, and the window offers picking only while idle too.
//!
//! A point saved from a pick keeps the screens it was picked on. Clicking
//! at it starts only on those screens, and stops when they change: on other
//! screens the same numbers are another place.

use super::{Controller, Job, Platform, State};
use crate::clicker::Position;
use crate::message::{Hindrance, Notice, PointScreens, Start, Stop, Unavailability};
use crate::recording::{Layout, Point, Rect};
use crate::window::{Located, Outcome};

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

    /// Where the clicker clicks for `point`, picked on the output at `on`.
    pub(super) fn locate(&self, point: Point, on: Rect) -> Outcome {
        if !self.settled() {
            return Outcome::Busy;
        }
        match self.platform.layout() {
            None => Outcome::Unavailable(Unavailability::ScreensUnknown),
            Some(screens) => located(screens, point, on),
        }
    }
}

/// `point` on `screens`, when the output at `on` is still one of them and
/// holds it.
fn located(screens: Layout, point: Point, on: Rect) -> Outcome {
    let still_there = screens.outputs().iter().any(|output| output.rect() == on);
    if !still_there || !on.contains(point) {
        return Outcome::Unavailable(Unavailability::ScreensChanged);
    }
    let at = screens.absolute(point);
    Outcome::Located(Located {
        x: at.x,
        y: at.y,
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

    #[test]
    fn a_point_on_either_screen_maps_by_the_formula() {
        let panel = rect(0, 0, 2048, 1280);
        let dp1 = rect(2048, 0, 2293, 960);
        let cases = [
            // round(1024 * 65536 / 4341) = 15459; round(640 * 65536 / 1280) = 32768.
            (Point { x: 1024, y: 640 }, panel, at(15459, 32768)),
            // The panel's last pixel: round(2047 * 65536 / 4341) = 30904,
            // round(1279 * 65536 / 1280) = 65485.
            (Point { x: 2047, y: 1279 }, panel, at(30904, 65485)),
            // DP-1's first pixel: round(2048 * 65536 / 4341) = 30919.
            (Point { x: 2048, y: 0 }, dp1, at(30919, 0)),
            // DP-1's last pixel: round(4340 * 65536 / 4341) = 65521,
            // round(959 * 65536 / 1280) = 49101.
            (Point { x: 4340, y: 959 }, dp1, at(65521, 49101)),
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
            (Point { x: 3000, y: 960 }, rect(2048, 0, 2293, 960)),
            // On the panel, but said to be picked on DP-1.
            (Point { x: 10, y: 10 }, rect(2048, 0, 2293, 960)),
            // DP-1 as KScreen rounds it, 2294 wide: not KWin's screen.
            (Point { x: 2100, y: 10 }, rect(2048, 0, 2294, 960)),
            // A screen that is not there.
            (Point { x: 10, y: 10 }, rect(0, 0, 1920, 1080)),
        ];
        for (point, on) in cases {
            assert_eq!(located(screens(), point, on), changed, "{point:?} {on:?}");
        }
    }
}
