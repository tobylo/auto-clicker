//! The config file as TOML spells it, before the checks that span fields.
//!
//! Every field that can only be one thing parses straight into it here, with
//! an error that says what was expected; the either-or fields (`key`,
//! `position`, `repeat`) have their own parsers for the same reason. Numbers
//! that are checked against other fields keep their place in the file
//! (`Spanned`), so those errors name the line too.

use std::fmt;
use std::num::NonZeroU32;

use serde::Deserialize;
use serde::de::{self, MapAccess, Visitor};
use toml::Spanned;

use super::{HotkeyRoute, OutputRoute};
use crate::clicker::{Mode, Position, Repeat};
use crate::input::{AbsolutePosition, Button, Key};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawConfig {
    pub(super) version: Spanned<i64>,
    pub(super) tray_start_delay: Option<Spanned<i64>>,
    #[serde(default)]
    pub(super) clicker: RawClicker,
    #[serde(default)]
    pub(super) hotkeys: RawHotkeys,
    #[serde(default)]
    pub(super) input: RawInput,
}

/// `[input]`: which route synthetic input and hotkeys take.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawInput {
    pub(super) output: Option<RouteName>,
    pub(super) hotkeys: Option<RouteName>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RouteName {
    Kernel,
    Portal,
}

impl From<RouteName> for OutputRoute {
    fn from(name: RouteName) -> OutputRoute {
        match name {
            RouteName::Kernel => OutputRoute::Kernel,
            RouteName::Portal => OutputRoute::Portal,
        }
    }
}

impl From<RouteName> for HotkeyRoute {
    fn from(name: RouteName) -> HotkeyRoute {
        match name {
            RouteName::Kernel => HotkeyRoute::Kernel,
            RouteName::Portal => HotkeyRoute::Portal,
        }
    }
}

/// `[hotkeys]`: each action's binding, such as "Ctrl+F6", parsed after the
/// file so a bad one is reported at its line.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawHotkeys {
    pub(super) toggle_clicking: Option<Spanned<String>>,
    pub(super) toggle_recording: Option<Spanned<String>>,
    pub(super) toggle_playback: Option<Spanned<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawClicker {
    pub(super) mode: Option<ModeName>,
    pub(super) interval_ms: Option<Spanned<i64>>,
    pub(super) clicks_per_second: Option<Spanned<f64>>,
    pub(super) jitter_percent: Option<Spanned<i64>>,
    pub(super) button: Option<ButtonName>,
    pub(super) key: Option<KeySpec>,
    pub(super) position: Option<PositionSpec>,
    pub(super) repeat: Option<RepeatSpec>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ModeName {
    Click,
    DoubleClick,
    HoldButton,
    KeyRepeat,
    HoldKey,
}

impl From<ModeName> for Mode {
    fn from(name: ModeName) -> Mode {
        match name {
            ModeName::Click => Mode::Click,
            ModeName::DoubleClick => Mode::DoubleClick,
            ModeName::HoldButton => Mode::HoldButton,
            ModeName::KeyRepeat => Mode::KeyRepeat,
            ModeName::HoldKey => Mode::HoldKey,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ButtonName {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl From<ButtonName> for Button {
    fn from(name: ButtonName) -> Button {
        match name {
            ButtonName::Left => Button::Left,
            ButtonName::Right => Button::Right,
            ButtonName::Middle => Button::Middle,
            ButtonName::Back => Button::Side,
            ButtonName::Forward => Button::Extra,
        }
    }
}

/// `key`: a key name, or a Linux key code for a key without one.
#[derive(Debug)]
pub(super) struct KeySpec(pub(super) Key);

impl<'de> Deserialize<'de> for KeySpec {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(KeyVisitor)
    }
}

struct KeyVisitor;

impl Visitor<'_> for KeyVisitor {
    type Value = KeySpec;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a key name such as \"space\", \"w\" or \"f6\", or a key code from 1 to 248")
    }

    fn visit_str<E: de::Error>(self, name: &str) -> Result<KeySpec, E> {
        Key::from_name(name).map(KeySpec).ok_or_else(|| {
            E::custom(format!(
                "unknown key name {name:?}; names are lowercase, such as \"space\", \"w\" or \"f6\""
            ))
        })
    }

    fn visit_i64<E: de::Error>(self, code: i64) -> Result<KeySpec, E> {
        u16::try_from(code)
            .ok()
            .and_then(Key::new)
            .map(KeySpec)
            .ok_or_else(|| E::custom(format!("key code {code} is not from 1 to 248")))
    }
}

/// `position`: "cursor", or a fixed point `{ x = …, y = … }`.
#[derive(Debug)]
pub(super) struct PositionSpec(pub(super) Position);

impl<'de> Deserialize<'de> for PositionSpec {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(PositionVisitor)
    }
}

struct PositionVisitor;

impl<'de> Visitor<'de> for PositionVisitor {
    type Value = PositionSpec;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"cursor\" or a point { x = 0..65535, y = 0..65535 }")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<PositionSpec, E> {
        match value {
            "cursor" => Ok(PositionSpec(Position::Cursor)),
            _ => Err(E::invalid_value(de::Unexpected::Str(value), &self)),
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<PositionSpec, A::Error> {
        let point = Point::deserialize(de::value::MapAccessDeserializer::new(map))?;
        Ok(PositionSpec(Position::Fixed(AbsolutePosition {
            x: point.x.0,
            y: point.y.0,
        })))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Point {
    x: Coordinate,
    y: Coordinate,
}

/// One axis of a fixed point, 0 at the left or top of the output layout and
/// 65535 at the right or bottom.
struct Coordinate(u16);

impl<'de> Deserialize<'de> for Coordinate {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = i64::deserialize(deserializer)?;
        u16::try_from(value).map(Coordinate).map_err(|_| {
            de::Error::custom(format!(
                "coordinate {value} is not from 0 (left or top) to 65535 (right or bottom)"
            ))
        })
    }
}

/// `repeat`: "until_stopped", or a number of actions from 1.
#[derive(Debug)]
pub(super) struct RepeatSpec(pub(super) Repeat);

impl<'de> Deserialize<'de> for RepeatSpec {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(RepeatVisitor)
    }
}

struct RepeatVisitor;

impl Visitor<'_> for RepeatVisitor {
    type Value = RepeatSpec;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"until_stopped\" or a number of actions from 1")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<RepeatSpec, E> {
        match value {
            "until_stopped" => Ok(RepeatSpec(Repeat::UntilStopped)),
            _ => Err(E::invalid_value(de::Unexpected::Str(value), &self)),
        }
    }

    fn visit_i64<E: de::Error>(self, count: i64) -> Result<RepeatSpec, E> {
        u32::try_from(count)
            .ok()
            .and_then(NonZeroU32::new)
            .map(|count| RepeatSpec(Repeat::Times(count)))
            .ok_or_else(|| {
                E::custom(format!(
                    "repeat {count} is not from 1 to {}; use \"until_stopped\" to repeat until stopped",
                    u32::MAX
                ))
            })
    }
}
