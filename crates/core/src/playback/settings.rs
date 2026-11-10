//! How a recording plays: how fast, and how many times.

use std::fmt;
use std::num::NonZeroU32;

use crate::clicker::Repeat;

/// How fast a recording plays, as a multiple of the speed it was recorded
/// at: 0.25 to 4. Kept in thousandths, so settings compare exactly and the
/// schedule is integer arithmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Speed(u16);

impl Speed {
    /// A quarter of the recorded speed.
    pub const SLOWEST: Speed = Speed(250);
    /// The recorded speed.
    pub const RECORDED: Speed = Speed(1000);
    /// Four times the recorded speed.
    pub const FASTEST: Speed = Speed(4000);

    /// The speed `ratio` times the recorded one, rounded to a thousandth;
    /// `None` unless it is a number from 0.25 to 4.
    pub fn from_ratio(ratio: f64) -> Option<Speed> {
        let thousandths = (ratio * 1000.0).round();
        let range = f64::from(Speed::SLOWEST.0)..=f64::from(Speed::FASTEST.0);
        // NaN is in no range. The cast cannot truncate: the value is a whole
        // number from 250 to 4000.
        range
            .contains(&thousandths)
            .then_some(Speed(thousandths as u16))
    }

    /// The speed in thousandths of the recorded one: 1000 for 1.0.
    pub fn thousandths(self) -> u16 {
        self.0
    }
}

impl Default for Speed {
    fn default() -> Self {
        Speed::RECORDED
    }
}

impl fmt::Display for Speed {
    /// "1", "1.5" or "0.25": no trailing zeros.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (whole, part) = (self.0 / 1000, self.0 % 1000);
        if part == 0 {
            return write!(f, "{whole}");
        }
        let digits = format!("{part:03}");
        write!(f, "{whole}.{}", digits.trim_end_matches('0'))
    }
}

/// `[playback]`: how a recording plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackSettings {
    pub speed: Speed,
    /// How many times the recording plays. Each time keeps the recording's
    /// idle time at its end, so the next one starts a whole recording later.
    pub repeat: Repeat,
}

impl Default for PlaybackSettings {
    /// Once, at the recorded speed.
    fn default() -> Self {
        PlaybackSettings {
            speed: Speed::RECORDED,
            repeat: Repeat::Times(NonZeroU32::MIN),
        }
    }
}
