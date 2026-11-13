//! Why the settings window's request was not done.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{NoScreens, Quoted, Unplayable};
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
    /// The screen layout is not known, for this reason, so no point can be
    /// picked.
    ScreensUnknown(NoScreens),
    /// The screens changed since the point was picked: pick it again.
    ScreensChanged,
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
            Unavailability::ScreensUnknown(why) => {
                write!(f, "the screen layout is not known: {why}")
            }
            Unavailability::ScreensChanged => {
                f.write_str("the screens changed while the point was picked: pick it again")
            }
        }
    }
}

/// What the settings window asked to change about a recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingChange {
    Rename,
    Delete,
}

impl RecordingChange {
    /// "Renamed", "Deleted".
    pub fn done(self) -> &'static str {
        match self {
            RecordingChange::Rename => "Renamed",
            RecordingChange::Delete => "Deleted",
        }
    }

    /// "Not renamed", "Not deleted".
    pub fn not_done(self) -> &'static str {
        match self {
            RecordingChange::Rename => "Not renamed",
            RecordingChange::Delete => "Not deleted",
        }
    }
}

/// Why a recording was not changed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ChangeFailure {
    /// There is no such file, or no recordings folder.
    Missing,
    /// The new name is not one a recording may have: the check's words.
    Refused(Quoted),
    /// The file could not be read, or is not a valid recording.
    Unreadable(Unplayable),
    /// The recordings folder could not be opened.
    Folder(Quoted),
    /// Writing or removing the file failed.
    NotDone { step: Step, why: Quoted },
    /// The renamed recording would be larger than this many MiB.
    TooLarge { mib: u64 },
}

/// What failed to happen to a recording's file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Written,
    Deleted,
}

impl fmt::Display for ChangeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeFailure::Missing => f.write_str("the file is no longer there"),
            ChangeFailure::Refused(why) | ChangeFailure::Folder(why) => why.fmt(f),
            ChangeFailure::Unreadable(why) => why.fmt(f),
            ChangeFailure::NotDone { step, why } => {
                let done = match step {
                    Step::Written => "written",
                    Step::Deleted => "deleted",
                };
                write!(f, "the file could not be {done}: {why}")
            }
            ChangeFailure::TooLarge { mib } => write!(
                f,
                "the file could not be written: the renamed recording would be larger than \
                 {mib} MiB"
            ),
        }
    }
}
