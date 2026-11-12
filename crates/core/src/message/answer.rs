//! Why the settings window's request was not done.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::Quoted;
use crate::controller::Blocked;
use crate::hotkeys::Unavailable;

/// Why a save or the desktop's settings failed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Failure {
    /// Neither `XDG_CONFIG_HOME` nor `HOME` is set.
    NoConfigDirectory,
    /// Writing the config file failed.
    Write(Quoted),
    /// The window's activation token is not one.
    BadToken,
    /// The desktop did not open its settings: its words.
    Desktop(Quoted),
    /// The desktop did not open its settings in time.
    NotOpened,
    /// Hotkeys stopped working while the desktop's settings were asked for.
    Hotkeys(Unavailable),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::NoConfigDirectory => {
                f.write_str("there is no config directory: neither XDG_CONFIG_HOME nor HOME is set")
            }
            Failure::Write(why) => write!(f, "the config file could not be written: {why}"),
            Failure::BadToken => f.write_str("the activation token is not one"),
            Failure::Desktop(why) => why.fmt(f),
            Failure::NotOpened => f.write_str("the desktop did not open its settings"),
            Failure::Hotkeys(why) => why.fmt(f),
        }
    }
}

/// Why a capture or the desktop's settings cannot be had here.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Unavailability {
    /// The desktop delivers the hotkeys, so the program cannot capture one.
    DesktopDelivers,
    /// The session may not be read.
    Blocked(Blocked),
    /// The program reads the keyboards, so the desktop has no settings for
    /// its hotkeys.
    ReadsKeyboards,
    /// The desktop's shortcuts cannot be reached.
    NoShortcuts,
    /// Hotkeys cannot be read.
    Hotkeys(Unavailable),
}

impl fmt::Display for Unavailability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unavailability::DesktopDelivers => f.write_str(
                "the desktop delivers the hotkeys: change them in System Settings, under Shortcuts",
            ),
            Unavailability::Blocked(why) => why.fmt(f),
            Unavailability::ReadsKeyboards => {
                f.write_str("the program reads the keyboards: set the hotkeys in this window")
            }
            Unavailability::NoShortcuts => f.write_str("the desktop's shortcuts are not available"),
            Unavailability::Hotkeys(why) => why.fmt(f),
        }
    }
}
