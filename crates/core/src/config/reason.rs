//! Why a value is not a valid setting, as a value: the config file shows
//! its English text, and the settings window its own language.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::hotkeys::Binding;
use crate::message::Quoted;
use crate::playback::Speed;

/// Why a setting is refused, with the limits it broke.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The tray's start delay is over `max` seconds.
    TrayStartDelay { max: u64 },
    /// The interval in milliseconds is outside `floor` to `max`; `double`
    /// when the floor is a double click's.
    Interval { floor: u64, max: u64, double: bool },
    /// The rate is outside what the intervals `floor` to `max`
    /// milliseconds give: see [`Reason::rates`].
    Rate { floor: u64, max: u64, double: bool },
    /// The jitter is over `max` percent.
    Jitter { max: u8 },
    /// The speed is outside [`Speed::SLOWEST`] to [`Speed::FASTEST`].
    Speed,
    /// The key code is not one.
    KeyCode { code: i64 },
    /// The count is not from 1 to `u32::MAX`.
    Count { count: i64 },
    /// Two actions have this hotkey.
    BoundTwice(Binding),
    /// This hotkey uses the key the clicker presses.
    OnClickerKey(Binding),
    /// The hotkey is not one: the parser's words.
    Binding(Quoted),
    /// The desktop chooses the hotkeys, in its own settings.
    DesktopChooses,
}

impl Reason {
    /// The fewest and the most actions a second that intervals of `floor`
    /// to `max` milliseconds give.
    pub fn rates(floor: u64, max: u64) -> (f64, f64) {
        // Shown rounded: the exact limits are 1000/floor and 1000/max.
        (1000.0 / max as f64, 1000.0 / floor as f64)
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::TrayStartDelay { max } => {
                write!(f, "tray_start_delay must be from 0 to {max} seconds")
            }
            Reason::Interval { floor, max, double } => write!(
                f,
                "interval_ms must be from {floor} to {max}{}",
                for_double(*double)
            ),
            Reason::Rate { floor, max, double } => {
                let (least, most) = Reason::rates(*floor, *max);
                write!(
                    f,
                    "clicks_per_second must be from {least:.6} to {most}{}",
                    for_double(*double)
                )
            }
            Reason::Jitter { max } => write!(f, "jitter_percent must be from 0 to {max}"),
            Reason::Speed => write!(
                f,
                "speed must be from {} to {}",
                Speed::SLOWEST,
                Speed::FASTEST
            ),
            Reason::KeyCode { code } => write!(f, "key code {code} is not from 1 to 248"),
            Reason::Count { count } => write!(
                f,
                "repeat {count} is not from 1 to {}; use \"until_stopped\" to repeat until \
                 stopped",
                u32::MAX
            ),
            Reason::BoundTwice(binding) => write!(f, "hotkey {binding} is bound to two actions"),
            Reason::OnClickerKey(binding) => write!(
                f,
                "hotkey {binding} uses the key the clicker presses; choose another key"
            ),
            Reason::Binding(why) => why.fmt(f),
            Reason::DesktopChooses => f.write_str(
                "the desktop delivers the hotkeys: change them in System Settings, under Shortcuts",
            ),
        }
    }
}

fn for_double(double: bool) -> &'static str {
    if double {
        " for a double click, which fits two clicks in each interval"
    } else {
        ""
    }
}
