//! The config as the settings window edits it: every setting spelled as the
//! file spells it, typed, and checked by the same rules as the file. The
//! window sends one in a save; the daemon turns it into a [`Config`] or says
//! which field is wrong.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::names::{ButtonName, ModeName, RouteName};
use super::{Config, HotkeyRoute, InputSettings, check};
use crate::clicker::{ClickerSettings, Interval, Mode, Position, Repeat};
use crate::hotkeys::{Action, Binding, Bindings};
use crate::input::AbsolutePosition;
use crate::playback::PlaybackSettings;

/// Every setting of the config file.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    /// Seconds.
    pub tray_start_delay: u64,
    pub clicker: ClickerDraft,
    pub hotkeys: HotkeysDraft,
    pub input: InputDraft,
    pub playback: PlaybackDraft,
}

/// `[clicker]`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClickerDraft {
    pub mode: ModeName,
    pub interval: IntervalDraft,
    pub jitter_percent: u64,
    pub button: ButtonName,
    /// A Linux key code.
    pub key: u16,
    pub position: PositionDraft,
    pub repeat: CountDraft,
}

/// The interval, as `interval_ms` or as `clicks_per_second`.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum IntervalDraft {
    Ms(u64),
    PerSecond(f64),
}

/// `position`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PositionDraft {
    Cursor,
    Fixed { x: u16, y: u16 },
}

/// A `repeat`: until stopped, or a count from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CountDraft {
    UntilStopped,
    Times(u64),
}

/// `[hotkeys]`, each as a binding such as "Ctrl+Shift+F6".
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HotkeysDraft {
    pub toggle_clicking: String,
    pub toggle_recording: String,
    pub toggle_playback: String,
}

/// `[input]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputDraft {
    pub output: RouteName,
    pub hotkeys: RouteName,
}

/// `[playback]`.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybackDraft {
    /// Times the recorded speed.
    pub speed: f64,
    pub repeat: CountDraft,
}

/// The setting a refused value belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    TrayStartDelay,
    Interval,
    Jitter,
    Key,
    Repeat,
    Hotkey(Action),
    Speed,
    PlaybackRepeat,
}

/// Why a draft is not a valid config: the field, and the reason.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Refusal {
    pub field: Field,
    pub reason: String,
}

impl Refusal {
    pub fn new(field: Field, reason: String) -> Refusal {
        Refusal { field, reason }
    }
}

/// A number the window sent, as the file's checks take it. A number too
/// large for them is refused by them anyway.
fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

impl Draft {
    /// The config this draft spells, checked as the file is, or the first
    /// field that is wrong.
    pub fn to_config(&self) -> Result<Config, Refusal> {
        let at = |field| move |reason| Refusal::new(field, reason);
        let tray_start_delay = check::tray_start_delay(signed(self.tray_start_delay))
            .map_err(at(Field::TrayStartDelay))?;
        let clicker = self.clicker.to_settings()?;
        let input = InputSettings {
            output: self.input.output.into(),
            hotkeys: self.input.hotkeys.into(),
        };
        let hotkeys = self.hotkeys.to_bindings()?;
        if let Some((_, later)) = check::duplicate(&hotkeys) {
            let reason = check::bound_twice(hotkeys.binding(later));
            return Err(Refusal::new(Field::Hotkey(later), reason));
        }
        if input.hotkeys == HotkeyRoute::Kernel
            && let Some((action, reason)) = check::on_the_clicker_key(&hotkeys, &clicker)
        {
            return Err(Refusal::new(Field::Hotkey(action), reason));
        }
        let playback = PlaybackSettings {
            speed: check::speed(self.playback.speed).map_err(at(Field::Speed))?,
            repeat: self
                .playback
                .repeat
                .to_repeat()
                .map_err(at(Field::PlaybackRepeat))?,
        };
        Ok(Config {
            tray_start_delay,
            clicker,
            hotkeys,
            input,
            playback,
        })
    }
}

impl ClickerDraft {
    fn to_settings(&self) -> Result<ClickerSettings, Refusal> {
        let at = |field| move |reason| Refusal::new(field, reason);
        let mode = Mode::from(self.mode);
        let interval = match self.interval {
            IntervalDraft::Ms(ms) => check::interval_ms(signed(ms), mode),
            IntervalDraft::PerSecond(rate) => check::clicks_per_second(rate, mode),
        }
        .map_err(at(Field::Interval))?;
        Ok(ClickerSettings {
            mode,
            interval,
            jitter: check::jitter_percent(signed(self.jitter_percent))
                .map_err(at(Field::Jitter))?,
            button: self.button.into(),
            key: check::key_code(i64::from(self.key)).map_err(at(Field::Key))?,
            position: match self.position {
                PositionDraft::Cursor => Position::Cursor,
                PositionDraft::Fixed { x, y } => Position::Fixed(AbsolutePosition { x, y }),
            },
            repeat: self.repeat.to_repeat().map_err(at(Field::Repeat))?,
        })
    }
}

impl HotkeysDraft {
    fn to_bindings(&self) -> Result<Bindings, Refusal> {
        let parse = |action, text: &str| {
            Binding::parse(text).map_err(|reason| Refusal::new(Field::Hotkey(action), reason))
        };
        Ok(Bindings {
            toggle_clicking: parse(Action::ToggleClicking, &self.toggle_clicking)?,
            toggle_recording: parse(Action::ToggleRecording, &self.toggle_recording)?,
            toggle_playback: parse(Action::TogglePlayback, &self.toggle_playback)?,
        })
    }
}

impl CountDraft {
    fn to_repeat(self) -> Result<Repeat, String> {
        match self {
            CountDraft::UntilStopped => Ok(Repeat::UntilStopped),
            CountDraft::Times(count) => check::count(signed(count)).map(Repeat::Times),
        }
    }
}

impl From<Repeat> for CountDraft {
    fn from(repeat: Repeat) -> CountDraft {
        match repeat {
            Repeat::UntilStopped => CountDraft::UntilStopped,
            Repeat::Times(count) => CountDraft::Times(u64::from(count.get())),
        }
    }
}

impl From<&Config> for Draft {
    fn from(config: &Config) -> Draft {
        let clicker = &config.clicker;
        let bindings = &config.hotkeys;
        Draft {
            tray_start_delay: config.tray_start_delay.as_secs(),
            clicker: ClickerDraft {
                mode: clicker.mode.into(),
                interval: spell_interval(clicker.interval),
                jitter_percent: u64::from(clicker.jitter.percent()),
                button: clicker.button.into(),
                key: clicker.key.code(),
                position: match clicker.position {
                    Position::Cursor => PositionDraft::Cursor,
                    Position::Fixed(AbsolutePosition { x, y }) => PositionDraft::Fixed { x, y },
                },
                repeat: clicker.repeat.into(),
            },
            hotkeys: HotkeysDraft {
                toggle_clicking: bindings.toggle_clicking.to_string(),
                toggle_recording: bindings.toggle_recording.to_string(),
                toggle_playback: bindings.toggle_playback.to_string(),
            },
            input: InputDraft {
                output: config.input.output.into(),
                hotkeys: config.input.hotkeys.into(),
            },
            playback: PlaybackDraft {
                speed: f64::from(config.playback.speed.thousandths()) / 1000.0,
                repeat: config.playback.repeat.into(),
            },
        }
    }
}

/// The interval in whole milliseconds when it is one, else as the clicks
/// per second that read back to exactly this interval.
pub fn spell_interval(interval: Interval) -> IntervalDraft {
    let duration = interval.get();
    match u64::try_from(duration.as_millis()) {
        Ok(ms) if duration.subsec_nanos().is_multiple_of(1_000_000) => IntervalDraft::Ms(ms),
        _ => IntervalDraft::PerSecond(rate_for(duration)),
    }
}

/// The rate whose interval, computed as the file's `clicks_per_second` is,
/// is exactly `interval`: its reciprocal. Reading it back rounds to the
/// nearest nanosecond, and over every interval allowed (10 ms to a day)
/// the two divisions are off by less than 0.05 ns, so it reads back to
/// `interval` itself.
fn rate_for(interval: Duration) -> f64 {
    1.0 / interval.as_secs_f64()
}

#[cfg(test)]
mod tests;
