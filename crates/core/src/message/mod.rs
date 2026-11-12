//! What the program tells the user, as values: what happened, with its
//! values, and nothing yet in any language.
//!
//! The controller decides what to say and puts it in the view; the tray and
//! the settings window render it in the user's language. Each value's
//! `Display` is its English text. Text from outside the program (an I/O
//! error, a portal's reason, a parser's message) is [`Quoted`] as it is.
//!
//! The reasons inside these messages come from their own modules, each
//! with its English text: [`Blocked`], [`Unavailable`](crate::hotkeys::Unavailable),
//! [`Refusal`](crate::hotkeys::Refusal) (a key the desktop's hotkeys use),
//! [`NoRecording`](crate::controller::NoRecording), and the settings'
//! [`Reason`](crate::config::Reason).

mod answer;
mod input;
mod notice;
mod quoted;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use answer::{Failure, Unavailability};
pub use input::InputFailure;
pub use notice::{Hindrance, Notice, PlayStart, SaveFailure, Saving, Start, Stop, Unplayable};
pub use quoted::Quoted;

use crate::config::ConfigError;
use crate::controller::Blocked;
use crate::hotkeys::Unavailable;
use crate::window::bounded;

/// What is wrong now, as the tray and the settings window list it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Problem {
    /// The config could not be used; the program runs on the defaults.
    Config(ConfigProblem),
    /// Hotkeys cannot be read.
    Hotkeys(Unavailable),
    /// The session lets nothing start.
    Blocked(Blocked),
    /// The devices could not be opened.
    Input(InputFailure),
    /// The recordings could not be listed.
    Unlisted(Quoted),
}

impl Problem {
    /// The problem with the text from outside the program that it does not
    /// quote as [`Quoted`] cut, as [`Quoted`] is cut on its way to the
    /// window.
    pub fn bounded(self) -> Problem {
        match self {
            Problem::Hotkeys(why) => Problem::Hotkeys(bounded_unavailable(why)),
            Problem::Blocked(why) => Problem::Blocked(bounded_blocked(why)),
            other => other,
        }
    }
}

/// `why` with its quoted text cut, for the window.
fn bounded_blocked(why: Blocked) -> Blocked {
    match why {
        Blocked::Unknown(why) => Blocked::Unknown(bounded(&why)),
        other => other,
    }
}

/// `why` with its quoted text cut, for the window.
pub fn bounded_unavailable(why: Unavailable) -> Unavailable {
    match why {
        Unavailable::ReadFailed(why) => Unavailable::ReadFailed(bounded(&why)),
        Unavailable::WatchFailed(why) => Unavailable::WatchFailed(bounded(&why)),
        Unavailable::Desktop(why) => Unavailable::Desktop(bounded(&why)),
        other => other,
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Config(problem) => problem.fmt(f),
            Problem::Hotkeys(why) => why.fmt(f),
            Problem::Blocked(why) => write!(f, "Clicking cannot start: {why}"),
            Problem::Input(why) => write!(f, "Clicking cannot start: {why}"),
            Problem::Unlisted(why) => write!(f, "The recordings cannot be listed: {why}"),
        }
    }
}

/// Why the config could not be used. What a parser said stays its own
/// words.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfigProblem {
    /// Neither `XDG_CONFIG_HOME` nor `HOME` is set.
    NoDirectory,
    /// The file could not be read.
    Unreadable { file: Quoted, why: Quoted },
    /// The file is not a valid config: where, from line 1, and why.
    Invalid {
        file: Quoted,
        line: Option<usize>,
        why: Quoted,
    },
}

impl From<&ConfigError> for ConfigProblem {
    fn from(err: &ConfigError) -> ConfigProblem {
        let file = Quoted::new(err.path().display().to_string());
        match err.problem() {
            crate::config::Problem::Read(why) => ConfigProblem::Unreadable {
                file,
                why: Quoted::new(why.to_string()),
            },
            crate::config::Problem::Invalid { line, reason } => ConfigProblem::Invalid {
                file,
                line: *line,
                why: Quoted::new(reason.clone()),
            },
        }
    }
}

impl fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigProblem::NoDirectory => {
                f.write_str("no config directory: neither XDG_CONFIG_HOME nor HOME is set")
            }
            ConfigProblem::Unreadable { file, why } => {
                write!(f, "{file}: could not read the file: {why}")
            }
            ConfigProblem::Invalid {
                file,
                line: Some(line),
                why,
            } => write!(f, "{file}:{line}: {why}"),
            ConfigProblem::Invalid {
                file,
                line: None,
                why,
            } => write!(f, "{file}: {why}"),
        }
    }
}

#[cfg(test)]
mod tests;
