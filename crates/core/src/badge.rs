//! The badge the on-screen display shows while a run goes on: a rounded
//! tile in the job's colour with a white mark, a pointer while clicking, a
//! dot while recording and a triangle while playing (O01 decision 15).
//!
//! Nothing here draws a pixel: the marks are shapes on a tile one unit
//! wide, which the display samples as the tray icon samples its tile.

use crate::config::OnScreenSettings;
use crate::controller::Activity;
use crate::digits::{CLICKING, Point, RECORDING, Rgb};

/// The mark on the tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Clicking: a pointer arrow, its tip up and to the left.
    Pointer,
    /// Recording: a dot.
    Dot,
    /// Playing: a triangle pointing right.
    Triangle,
}

/// A badge: its mark, and the tile's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Badge {
    pub mark: Mark,
    pub tile: Rgb,
}

/// The mark's colour.
pub const MARK: Rgb = Rgb(0xff, 0xff, 0xff);

/// The pointer's outline, on a tile one unit wide.
const POINTER: [(f32, f32); 7] = [
    (0.33, 0.2),
    (0.33, 0.76),
    (0.46, 0.63),
    (0.555, 0.82),
    (0.64, 0.78),
    (0.545, 0.59),
    (0.72, 0.59),
];
/// The triangle, its centre of area near the tile's centre.
const TRIANGLE: [(f32, f32); 3] = [(0.37, 0.27), (0.37, 0.73), (0.75, 0.5)];
/// The dot's radius, around the tile's centre.
const DOT: f32 = 0.24;

/// The badge while `activity` runs, where `on_screen` switches it on;
/// `None` otherwise: idle, a countdown, and a start that waits for keys to
/// come up or for the devices to open, when the desktop's permission dialog
/// may be on the screen.
pub fn of(activity: Activity, on_screen: &OnScreenSettings) -> Option<Badge> {
    let (mark, tile) = match activity {
        Activity::Clicking if on_screen.clicking => (Mark::Pointer, CLICKING),
        Activity::Recording if on_screen.recording => (Mark::Dot, RECORDING),
        Activity::Playing if on_screen.playback => (Mark::Triangle, CLICKING),
        _ => return None,
    };
    Some(Badge { mark, tile })
}

impl Mark {
    /// Whether `point`, on a tile one unit wide, lies on the mark.
    pub fn contains(self, point: Point) -> bool {
        match self {
            Mark::Pointer => inside(&POINTER, point),
            Mark::Triangle => inside(&TRIANGLE, point),
            Mark::Dot => {
                let (dx, dy) = (point.x - 0.5, point.y - 0.5);
                dx * dx + dy * dy <= DOT * DOT
            }
        }
    }
}

/// Whether `point` lies inside the polygon `corners`, by the even-odd rule.
fn inside(corners: &[(f32, f32)], Point { x, y }: Point) -> bool {
    let mut odd = false;
    let edges = corners.iter().zip(corners.iter().cycle().skip(1));
    for (&(x1, y1), &(x2, y2)) in edges {
        if (y1 > y) != (y2 > y) && x < x1 + (y - y1) * (x2 - x1) / (y2 - y1) {
            odd = !odd;
        }
    }
    odd
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Corner;

    const ALL_ON: OnScreenSettings = OnScreenSettings {
        countdown: true,
        clicking: true,
        recording: true,
        playback: true,
        corner: Corner::TopLeft,
    };

    fn at(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    #[test]
    fn clicking_recording_and_playing_each_have_their_badge() {
        let badge = |activity| of(activity, &ALL_ON);
        let pointer = Badge {
            mark: Mark::Pointer,
            tile: CLICKING,
        };
        let dot = Badge {
            mark: Mark::Dot,
            tile: RECORDING,
        };
        let triangle = Badge {
            mark: Mark::Triangle,
            tile: CLICKING,
        };
        assert_eq!(badge(Activity::Clicking), Some(pointer));
        assert_eq!(badge(Activity::Recording), Some(dot));
        assert_eq!(badge(Activity::Playing), Some(triangle));
        for activity in [
            Activity::Idle,
            Activity::StartingIn(3),
            Activity::WaitingForRelease,
            Activity::Opening,
            Activity::RecordingIn(3),
            Activity::PlayingIn(3),
            Activity::PlayWaitingForRelease,
            Activity::PlayOpening,
        ] {
            assert_eq!(badge(activity), None, "{activity:?}");
        }
    }

    #[test]
    fn each_switch_turns_its_own_badge_off() {
        let off = [
            (
                Activity::Clicking,
                OnScreenSettings {
                    clicking: false,
                    ..ALL_ON
                },
            ),
            (
                Activity::Recording,
                OnScreenSettings {
                    recording: false,
                    ..ALL_ON
                },
            ),
            (
                Activity::Playing,
                OnScreenSettings {
                    playback: false,
                    ..ALL_ON
                },
            ),
        ];
        for (activity, switches) in off {
            assert_eq!(of(activity, &switches), None, "{activity:?}");
            // The other two still show theirs.
            for (other, _) in off.iter().filter(|(other, _)| *other != activity) {
                assert!(
                    of(*other, &switches).is_some(),
                    "{other:?} with {activity:?} off"
                );
            }
        }
        // The countdown's switch is not the badges'.
        let no_countdown = OnScreenSettings {
            countdown: false,
            ..ALL_ON
        };
        assert!(of(Activity::Clicking, &no_countdown).is_some());
    }

    #[test]
    fn each_mark_lies_inside_the_tile_around_its_middle() {
        for mark in [Mark::Pointer, Mark::Dot, Mark::Triangle] {
            // Nothing near the tile's edges, so the tile shows around it.
            for edge in [
                at(0.1, 0.5),
                at(0.9, 0.5),
                at(0.5, 0.1),
                at(0.5, 0.9),
                at(0.05, 0.05),
            ] {
                assert!(!mark.contains(edge), "{mark:?} at {edge:?}");
            }
        }
        assert!(Mark::Dot.contains(at(0.5, 0.5)));
        assert!(!Mark::Dot.contains(at(0.5 + DOT + 0.01, 0.5)));
        assert!(Mark::Triangle.contains(at(0.5, 0.5)));
        assert!(!Mark::Triangle.contains(at(0.36, 0.5)));
        assert!(!Mark::Triangle.contains(at(0.7, 0.3)));
        // The pointer: its body, its tail, and the notch between them.
        assert!(Mark::Pointer.contains(at(0.4, 0.45)));
        assert!(Mark::Pointer.contains(at(0.59, 0.75)));
        assert!(!Mark::Pointer.contains(at(0.45, 0.75)));
        assert!(!Mark::Pointer.contains(at(0.7, 0.5)));
    }
}
