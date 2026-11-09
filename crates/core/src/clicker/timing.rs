//! How long each part of an action takes: the jittered interval and the
//! shape of a click inside it.

use std::time::Duration;

use super::{ClickerSettings, Jitter, Mode};

/// The longest a click holds its button or key down.
const HOLD: Duration = Duration::from_millis(10);

/// The time from a double click's first press to its second.
const DOUBLE_CLICK_GAP: Duration = Duration::from_millis(30);

/// The timing of one action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Shape {
    /// How long each press is held before its release.
    pub(super) hold: Duration,
    /// For a double click, the time from the first press to the second.
    pub(super) second_press: Option<Duration>,
}

/// The interval before the next action: the configured one with jitter
/// applied, never below the mode's floor.
pub(super) fn next_interval(settings: &ClickerSettings, rng: &mut fastrand::Rng) -> Duration {
    jittered(settings.interval.get(), settings.jitter, rng).max(settings.mode.interval_floor())
}

/// `base` moved by a uniformly random amount within ± the jitter's percent.
fn jittered(base: Duration, jitter: Jitter, rng: &mut fastrand::Rng) -> Duration {
    // An interval is at most a day, about 8.6e13 ns, so neither the
    // conversion nor the products below come near u64::MAX; saturating
    // keeps that true for any input.
    let base_ns = u64::try_from(base.as_nanos()).unwrap_or(u64::MAX);
    let span = base_ns.saturating_mul(u64::from(jitter.percent())) / 100;
    if span == 0 {
        return base;
    }
    let lowest = base_ns.saturating_sub(span);
    Duration::from_nanos(lowest.saturating_add(rng.u64(0..=span.saturating_mul(2))))
}

/// The shape of an action that takes `interval`: every part fits inside it.
pub(super) fn shape(mode: Mode, interval: Duration) -> Shape {
    let half = interval / 2;
    match mode {
        Mode::DoubleClick => {
            let gap = DOUBLE_CLICK_GAP.min(half);
            Shape {
                hold: HOLD.min(gap / 2),
                second_press: Some(gap),
            }
        }
        _ => Shape {
            hold: HOLD.min(half),
            second_press: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clicker::{INTERVAL_FLOOR, Interval};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn settings(mode: Mode, interval_ms: u64, jitter: u8) -> ClickerSettings {
        ClickerSettings {
            mode,
            interval: Interval::new(ms(interval_ms)).unwrap(),
            jitter: Jitter::new(jitter).unwrap(),
            ..ClickerSettings::default()
        }
    }

    #[test]
    fn no_jitter_keeps_the_interval() {
        let mut rng = fastrand::Rng::with_seed(7);
        let settings = settings(Mode::Click, 100, 0);
        assert!((0..100).all(|_| next_interval(&settings, &mut rng) == ms(100)));
    }

    #[test]
    fn jitter_stays_within_its_bounds_and_reaches_both_sides() {
        let mut rng = fastrand::Rng::with_seed(7);
        let settings = settings(Mode::Click, 100, 20);
        let samples: Vec<_> = (0..1000)
            .map(|_| next_interval(&settings, &mut rng))
            .collect();
        assert!(samples.iter().all(|d| (ms(80)..=ms(120)).contains(d)));
        assert!(samples.iter().any(|d| *d < ms(90)));
        assert!(samples.iter().any(|d| *d > ms(110)));
    }

    #[test]
    fn jitter_never_goes_below_the_floor() {
        let mut rng = fastrand::Rng::with_seed(7);
        let settings = settings(Mode::Click, 12, 50);
        let samples: Vec<_> = (0..1000)
            .map(|_| next_interval(&settings, &mut rng))
            .collect();
        assert!(samples.iter().all(|d| *d >= INTERVAL_FLOOR));
        assert!(samples.contains(&INTERVAL_FLOOR));
    }

    #[test]
    fn a_double_click_interval_never_goes_below_twice_the_floor() {
        let mut rng = fastrand::Rng::with_seed(7);
        let settings = settings(Mode::DoubleClick, 20, 50);
        assert!((0..1000).all(|_| next_interval(&settings, &mut rng) >= INTERVAL_FLOOR * 2));
    }

    #[test]
    fn a_click_holds_ten_ms_or_half_a_short_interval() {
        assert_eq!(shape(Mode::Click, ms(100)).hold, ms(10));
        assert_eq!(shape(Mode::Click, ms(12)).hold, ms(6));
        assert_eq!(shape(Mode::KeyRepeat, ms(100)).second_press, None);
    }

    #[test]
    fn a_double_click_fits_both_clicks_in_its_interval() {
        assert_eq!(
            shape(Mode::DoubleClick, ms(100)),
            Shape {
                hold: ms(10),
                second_press: Some(ms(30))
            }
        );
        let short = shape(Mode::DoubleClick, ms(20));
        assert_eq!(short.second_press, Some(ms(10)));
        assert_eq!(short.hold, ms(5));
    }
}
