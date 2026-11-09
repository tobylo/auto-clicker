//! What a clicking run does: the validated settings the engine runs from.

use std::num::NonZeroU32;
use std::time::Duration;

use crate::input::{AbsolutePosition, Button, Key};

/// The shortest interval between two actions. Set by the click-rate
/// measurement recorded in docs/planning/map.md.
pub const INTERVAL_FLOOR: Duration = Duration::from_millis(10);

/// The longest interval between two actions: one day. Bounding it keeps every
/// deadline the engine computes far from overflowing.
pub const INTERVAL_MAX: Duration = Duration::from_secs(24 * 60 * 60);

/// The largest jitter, in percent of the interval.
pub const JITTER_MAX_PERCENT: u8 = 50;

/// What each action of a run does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Clicks the button once per interval.
    #[default]
    Click,
    /// Clicks the button twice per interval, as a double click.
    DoubleClick,
    /// Presses the button once and holds it until the run stops.
    HoldButton,
    /// Taps the key once per interval.
    KeyRepeat,
    /// Presses the key once and holds it until the run stops.
    HoldKey,
}

impl Mode {
    /// The shortest interval this mode runs at. A double click fits two clicks
    /// into one interval, so it needs twice the floor to keep each click at
    /// least [`INTERVAL_FLOOR`] apart.
    pub fn interval_floor(self) -> Duration {
        match self {
            Mode::DoubleClick => INTERVAL_FLOOR.saturating_mul(2),
            _ => INTERVAL_FLOOR,
        }
    }

    /// Whether the mode repeats on the interval, rather than holding.
    pub fn repeats(self) -> bool {
        !matches!(self, Mode::HoldButton | Mode::HoldKey)
    }
}

/// The time from one action's start to the next, within
/// [`INTERVAL_FLOOR`]`..=`[`INTERVAL_MAX`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Interval(Duration);

impl Interval {
    /// The interval, or `None` when it is outside the allowed range.
    pub fn new(interval: Duration) -> Option<Interval> {
        (INTERVAL_FLOOR..=INTERVAL_MAX)
            .contains(&interval)
            .then_some(Interval(interval))
    }

    pub fn get(self) -> Duration {
        self.0
    }
}

impl Default for Interval {
    /// Ten clicks a second.
    fn default() -> Self {
        Interval(Duration::from_millis(100))
    }
}

/// Random variation of each interval, uniform within ± this many percent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Jitter(u8);

impl Jitter {
    /// The jitter, or `None` above [`JITTER_MAX_PERCENT`].
    pub fn new(percent: u8) -> Option<Jitter> {
        (percent <= JITTER_MAX_PERCENT).then_some(Jitter(percent))
    }

    pub fn percent(self) -> u8 {
        self.0
    }
}

/// Where a button action happens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Position {
    /// Wherever the pointer is.
    #[default]
    Cursor,
    /// A point on the output layout; the pointer moves there before each
    /// action. Does nothing useful while a game has locked the pointer.
    Fixed(AbsolutePosition),
}

/// How many actions a run makes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    UntilStopped,
    /// This many actions, then the run ends by itself. A double click is one
    /// action.
    Times(NonZeroU32),
}

/// Everything a clicking run needs. Settings the mode does not use (the key
/// in a click mode, say) are kept, as remembered choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClickerSettings {
    pub mode: Mode,
    pub interval: Interval,
    pub jitter: Jitter,
    pub button: Button,
    pub key: Key,
    pub position: Position,
    pub repeat: Repeat,
}

/// The key a new configuration repeats or holds. Evaluated at compile time,
/// so the panic arm can only fail the build, never a run.
const DEFAULT_KEY: Key = match Key::new(57) {
    Some(key) => key,
    None => panic!("57 (KEY_SPACE) is a keyboard key"),
};

impl Default for ClickerSettings {
    /// Left clicks at the cursor, ten a second, until stopped.
    fn default() -> Self {
        ClickerSettings {
            mode: Mode::default(),
            interval: Interval::default(),
            jitter: Jitter::default(),
            button: Button::Left,
            key: DEFAULT_KEY,
            position: Position::default(),
            repeat: Repeat::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_accepts_exactly_floor_to_max() {
        let one = Duration::from_nanos(1);
        assert_eq!(Interval::new(INTERVAL_FLOOR - one), None);
        assert!(Interval::new(INTERVAL_FLOOR).is_some());
        assert!(Interval::new(INTERVAL_MAX).is_some());
        assert_eq!(Interval::new(INTERVAL_MAX + one), None);
    }

    #[test]
    fn jitter_accepts_up_to_fifty_percent() {
        assert_eq!(Jitter::new(50).map(Jitter::percent), Some(50));
        assert_eq!(Jitter::new(51), None);
    }

    #[test]
    fn double_click_needs_twice_the_floor() {
        assert_eq!(Mode::DoubleClick.interval_floor(), INTERVAL_FLOOR * 2);
        assert_eq!(Mode::Click.interval_floor(), INTERVAL_FLOOR);
        assert_eq!(Mode::KeyRepeat.interval_floor(), INTERVAL_FLOOR);
    }

    #[test]
    fn only_hold_modes_do_not_repeat() {
        let repeating = [Mode::Click, Mode::DoubleClick, Mode::KeyRepeat];
        assert!(repeating.iter().all(|mode| mode.repeats()));
        assert!(!Mode::HoldButton.repeats() && !Mode::HoldKey.repeats());
    }

    #[test]
    fn defaults_click_left_at_the_cursor_ten_times_a_second() {
        let settings = ClickerSettings::default();
        assert_eq!(settings.mode, Mode::Click);
        assert_eq!(settings.interval.get(), Duration::from_millis(100));
        assert_eq!(settings.button, Button::Left);
        assert_eq!(settings.key.name(), Some("space"));
        assert_eq!(settings.repeat, Repeat::UntilStopped);
    }
}
