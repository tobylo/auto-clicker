//! The checks that span fields or ranges, turning the raw file into a
//! [`Config`]. Each error points at the value it is about.

use std::fmt;
use std::ops::Range;
use std::time::Duration;

use toml::Spanned;

use super::check;
use super::raw::{RawClicker, RawConfig, RawPlayback};
use super::{Config, InputSettings};
use crate::clicker::{ClickerSettings, Interval, Mode};
use crate::playback::{PlaybackSettings, Speed};

/// The version of the file format this program reads.
pub(super) const VERSION: i64 = 1;

/// A problem with one value, and where in the file it is.
#[derive(Debug)]
pub(super) struct Invalid {
    pub(super) span: Option<Range<usize>>,
    pub(super) reason: String,
}

pub(super) fn at<T>(value: &Spanned<T>, reason: impl fmt::Display) -> Invalid {
    Invalid {
        span: Some(value.span()),
        reason: reason.to_string(),
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
    let fixed_screens = raw
        .clicker
        .position
        .as_ref()
        .and_then(|spec| spec.1.clone());
    let clicker = clicker(raw.clicker, defaults.clicker)?;
    let input = InputSettings {
        output: raw.input.output.map_or(defaults.input.output, Into::into),
        hotkeys: raw.input.hotkeys.map_or(defaults.input.hotkeys, Into::into),
    };
    Ok(Config {
        tray_start_delay,
        hotkeys: super::hotkeys::hotkeys(&raw.hotkeys, &clicker, input.hotkeys)?,
        clicker,
        fixed_screens,
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
    check::speed(*value.get_ref()).map_err(|reason| at(value, reason))
}

fn tray_start_delay(delay: &Spanned<i64>) -> Result<Duration, Invalid> {
    check::tray_start_delay(*delay.get_ref()).map_err(|reason| at(delay, reason))
}

fn clicker(raw: RawClicker, defaults: ClickerSettings) -> Result<ClickerSettings, Invalid> {
    let mode = raw.mode.map_or(defaults.mode, Mode::from);
    Ok(ClickerSettings {
        mode,
        interval: interval(&raw, mode)?.unwrap_or(defaults.interval),
        jitter: match &raw.jitter_percent {
            Some(jitter) => {
                check::jitter_percent(*jitter.get_ref()).map_err(|reason| at(jitter, reason))?
            }
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
        (Some(ms), None) => check::interval_ms(*ms.get_ref(), mode)
            .map(Some)
            .map_err(|reason| at(ms, reason)),
        (None, Some(rate)) => check::clicks_per_second(*rate.get_ref(), mode)
            .map(Some)
            .map_err(|reason| at(rate, reason)),
        (None, None) => Ok(None),
    }
}
