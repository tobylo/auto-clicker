//! The checks on each value, wherever it comes from: the config file, which
//! adds where the value is, or the settings window, which adds which field.
//! Each returns the validated value, or why it is not one.

use std::num::NonZeroU32;
use std::time::Duration;

use super::MAX_TRAY_START_DELAY;
use crate::clicker::{ClickerSettings, INTERVAL_MAX, Interval, JITTER_MAX_PERCENT, Jitter, Mode};
use crate::hotkeys::{Action, Binding, Bindings};
use crate::input::Key;
use crate::playback::Speed;

pub(crate) fn tray_start_delay(seconds: i64) -> Result<Duration, String> {
    let max = MAX_TRAY_START_DELAY.as_secs();
    u64::try_from(seconds)
        .ok()
        .filter(|seconds| *seconds <= max)
        .map(Duration::from_secs)
        .ok_or_else(|| format!("tray_start_delay must be from 0 to {max} seconds"))
}

pub(crate) fn interval_ms(ms: i64, mode: Mode) -> Result<Interval, String> {
    let (floor, max) = bounds_ms(mode);
    u64::try_from(ms)
        .ok()
        .and_then(|ms| for_mode_interval(Duration::from_millis(ms), mode))
        .ok_or_else(|| {
            format!(
                "interval_ms must be from {floor} to {max}{}",
                for_mode(mode)
            )
        })
}

pub(crate) fn clicks_per_second(rate: f64, mode: Mode) -> Result<Interval, String> {
    // A zero, negative or NaN rate gives an interval that is infinite,
    // negative or NaN, which `try_from_secs_f64` rejects; an infinite rate
    // gives zero, which is below the floor.
    Duration::try_from_secs_f64(1.0 / rate)
        .ok()
        .and_then(|interval| for_mode_interval(interval, mode))
        .ok_or_else(|| {
            let (floor, max) = bounds_ms(mode);
            // Shown rounded: the exact limits are 1000/floor and 1000/max.
            let most = 1000.0 / floor as f64;
            let least = 1000.0 / max as f64;
            format!(
                "clicks_per_second must be from {least:.6} to {most}{}",
                for_mode(mode)
            )
        })
}

/// `interval` when `mode` may run at it: not below the mode's floor, nor
/// outside the range every interval has.
fn for_mode_interval(interval: Duration, mode: Mode) -> Option<Interval> {
    Some(interval)
        .filter(|interval| *interval >= mode.interval_floor())
        .and_then(Interval::new)
}

/// The shortest and longest interval for `mode`, in whole milliseconds.
fn bounds_ms(mode: Mode) -> (u64, u64) {
    let ms = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
    (ms(mode.interval_floor()), ms(INTERVAL_MAX))
}

fn for_mode(mode: Mode) -> &'static str {
    match mode {
        Mode::DoubleClick => " for a double click, which fits two clicks in each interval",
        _ => "",
    }
}

pub(crate) fn jitter_percent(percent: i64) -> Result<Jitter, String> {
    u8::try_from(percent)
        .ok()
        .and_then(Jitter::new)
        .ok_or_else(|| format!("jitter_percent must be from 0 to {JITTER_MAX_PERCENT}"))
}

pub(crate) fn speed(ratio: f64) -> Result<Speed, String> {
    Speed::from_ratio(ratio).ok_or_else(|| {
        format!(
            "speed must be from {} to {}",
            Speed::SLOWEST,
            Speed::FASTEST
        )
    })
}

pub(crate) fn key_code(code: i64) -> Result<Key, String> {
    u16::try_from(code)
        .ok()
        .and_then(Key::new)
        .ok_or_else(|| format!("key code {code} is not from 1 to 248"))
}

/// A `repeat` count: actions or plays, from 1.
pub(crate) fn count(count: i64) -> Result<NonZeroU32, String> {
    u32::try_from(count)
        .ok()
        .and_then(NonZeroU32::new)
        .ok_or_else(|| {
            format!(
                "repeat {count} is not from 1 to {}; use \"until_stopped\" to repeat until stopped",
                u32::MAX
            )
        })
}

/// The first action, in [`Action::ALL`] order, whose binding an earlier
/// action already has; with that earlier action. Two actions on one binding
/// would both fire.
pub(crate) fn duplicate(bindings: &Bindings) -> Option<(Action, Action)> {
    let actions = bindings.actions();
    actions.iter().enumerate().find_map(|(index, later)| {
        actions[..index]
            .iter()
            .find(|earlier| earlier.1 == later.1)
            .map(|earlier| (earlier.0, later.0))
    })
}

pub(crate) fn bound_twice(binding: Binding) -> String {
    format!("hotkey {binding} is bound to two actions")
}

/// The first action whose binding uses the key the clicker presses.
///
/// A hotkey on the key the clicker repeats or holds could fire from the
/// clicker itself: a remapper such as keyd re-sends the clicker's keys from
/// its own device, and with a modifier the user happens to hold, they match
/// a binding with modifiers too. Holds for bindings the program matches
/// itself; where the desktop delivers the hotkeys, its own triggers decide.
pub(crate) fn on_the_clicker_key(
    bindings: &Bindings,
    clicker: &ClickerSettings,
) -> Option<(Action, String)> {
    let pressed = clicker.pressed_key()?;
    bindings
        .actions()
        .into_iter()
        .find(|(_, binding)| binding.key() == pressed)
        .map(|(action, binding)| {
            (
                action,
                format!("hotkey {binding} uses the key the clicker presses; choose another key"),
            )
        })
}
