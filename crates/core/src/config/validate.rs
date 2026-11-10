//! The checks that span fields or ranges, turning the raw file into a
//! [`Config`]. Each error points at the value it is about.

use std::ops::Range;
use std::time::Duration;

use toml::Spanned;

use super::raw::{RawClicker, RawConfig, RawPlayback};
use super::{Config, InputSettings, MAX_TRAY_START_DELAY};
use crate::clicker::{ClickerSettings, INTERVAL_MAX, Interval, JITTER_MAX_PERCENT, Jitter, Mode};
use crate::playback::{PlaybackSettings, Speed};

/// The version of the file format this program reads.
pub(super) const VERSION: i64 = 1;

/// A problem with one value, and where in the file it is.
#[derive(Debug)]
pub(super) struct Invalid {
    pub(super) span: Option<Range<usize>>,
    pub(super) reason: String,
}

pub(super) fn at<T>(value: &Spanned<T>, reason: String) -> Invalid {
    Invalid {
        span: Some(value.span()),
        reason,
    }
}

pub(super) fn validate(raw: RawConfig) -> Result<Config, Invalid> {
    let version = *raw.version.get_ref();
    if version != VERSION {
        return Err(at(
            &raw.version,
            format!("unsupported config version {version}; this program reads version {VERSION}"),
        ));
    }
    let defaults = Config::default();
    let tray_start_delay = match &raw.tray_start_delay {
        Some(delay) => tray_start_delay(delay)?,
        None => defaults.tray_start_delay,
    };
    let clicker = clicker(raw.clicker, defaults.clicker)?;
    let input = InputSettings {
        output: raw.input.output.map_or(defaults.input.output, Into::into),
        hotkeys: raw.input.hotkeys.map_or(defaults.input.hotkeys, Into::into),
    };
    Ok(Config {
        tray_start_delay,
        hotkeys: super::hotkeys::hotkeys(&raw.hotkeys, &clicker, input.hotkeys)?,
        clicker,
        input,
        playback: playback(&raw.playback, defaults.playback)?,
    })
}

fn playback(raw: &RawPlayback, defaults: PlaybackSettings) -> Result<PlaybackSettings, Invalid> {
    Ok(PlaybackSettings {
        speed: match &raw.speed {
            Some(written) => speed(written)?,
            None => defaults.speed,
        },
        repeat: raw.repeat.as_ref().map_or(defaults.repeat, |spec| spec.0),
    })
}

fn speed(value: &Spanned<f64>) -> Result<Speed, Invalid> {
    Speed::from_ratio(*value.get_ref()).ok_or_else(|| {
        at(
            value,
            format!(
                "speed must be from {} to {}",
                Speed::SLOWEST,
                Speed::FASTEST
            ),
        )
    })
}

fn tray_start_delay(delay: &Spanned<i64>) -> Result<Duration, Invalid> {
    let max = MAX_TRAY_START_DELAY.as_secs();
    u64::try_from(*delay.get_ref())
        .ok()
        .filter(|seconds| *seconds <= max)
        .map(Duration::from_secs)
        .ok_or_else(|| {
            at(
                delay,
                format!("tray_start_delay must be from 0 to {max} seconds"),
            )
        })
}

fn clicker(raw: RawClicker, defaults: ClickerSettings) -> Result<ClickerSettings, Invalid> {
    let mode = raw.mode.map_or(defaults.mode, Mode::from);
    Ok(ClickerSettings {
        mode,
        interval: interval(&raw, mode)?.unwrap_or(defaults.interval),
        jitter: match &raw.jitter_percent {
            Some(jitter) => jitter_percent(jitter)?,
            None => defaults.jitter,
        },
        button: raw.button.map_or(defaults.button, Into::into),
        key: raw.key.map_or(defaults.key, |spec| spec.0),
        position: raw.position.map_or(defaults.position, |spec| spec.0),
        repeat: raw.repeat.map_or(defaults.repeat, |spec| spec.0),
    })
}

/// The interval from whichever of `interval_ms` and `clicks_per_second` is
/// set, or `None` when neither is.
fn interval(raw: &RawClicker, mode: Mode) -> Result<Option<Interval>, Invalid> {
    match (&raw.interval_ms, &raw.clicks_per_second) {
        (Some(_), Some(rate)) => Err(at(
            rate,
            "set either interval_ms or clicks_per_second, not both".to_owned(),
        )),
        (Some(ms), None) => interval_ms(ms, mode).map(Some),
        (None, Some(rate)) => clicks_per_second(rate, mode).map(Some),
        (None, None) => Ok(None),
    }
}

fn interval_ms(value: &Spanned<i64>, mode: Mode) -> Result<Interval, Invalid> {
    let ms = *value.get_ref();
    let (floor, max) = bounds_ms(mode);
    let interval = u64::try_from(ms)
        .ok()
        .and_then(|ms| for_mode_interval(Duration::from_millis(ms), mode));
    interval.ok_or_else(|| {
        at(
            value,
            format!(
                "interval_ms must be from {floor} to {max}{}",
                for_mode(mode)
            ),
        )
    })
}

fn clicks_per_second(value: &Spanned<f64>, mode: Mode) -> Result<Interval, Invalid> {
    let rate = *value.get_ref();
    let (floor, max) = bounds_ms(mode);
    // A zero, negative or NaN rate gives an interval that is infinite,
    // negative or NaN, which `try_from_secs_f64` rejects; an infinite rate
    // gives zero, which is below the floor.
    let interval = Duration::try_from_secs_f64(1.0 / rate)
        .ok()
        .and_then(|interval| for_mode_interval(interval, mode));
    interval.ok_or_else(|| {
        // Shown rounded: the exact limits are 1000/floor and 1000/max.
        let most = 1000.0 / floor as f64;
        let least = 1000.0 / max as f64;
        at(
            value,
            format!(
                "clicks_per_second must be from {least:.6} to {most}{}",
                for_mode(mode)
            ),
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

fn jitter_percent(value: &Spanned<i64>) -> Result<Jitter, Invalid> {
    u8::try_from(*value.get_ref())
        .ok()
        .and_then(Jitter::new)
        .ok_or_else(|| {
            at(
                value,
                format!("jitter_percent must be from 0 to {JITTER_MAX_PERCENT}"),
            )
        })
}
