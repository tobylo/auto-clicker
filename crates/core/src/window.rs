//! What the settings window and the daemon say to each other over the
//! socket the daemon gives the window: one JSON object per line, each at
//! most [`MAX_LINE`] bytes.
//!
//! The window sends [`Request`]s. The daemon sends a [`WindowView`] at the
//! start and after every change: everything the window shows, including the
//! answer to its last request. Answers are part of the view rather than
//! messages of their own, so a newer view never hides one, and the daemon
//! only ever has the latest view to send.
//!
//! Each side parses what it reads once, into these types; anything else
//! (an unknown field, a line too long) ends the connection.

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::{Draft, InputDraft, Refusal, Stale};
use crate::controller::{Activity, Hotkey, NoRecording};
use crate::hotkeys::Action;

/// The longest line either side sends or reads, newline included.
pub const MAX_LINE: usize = 64 * 1024;

/// The most characters of one message (a problem, a refusal, a failure) a
/// view carries; a longer one is cut, so it can quote what it is about
/// (a hotkey the window sent, a line of the config file) and the view still
/// fits in [`MAX_LINE`], escaped as JSON escapes it.
pub const MAX_TEXT: usize = 500;

/// The most problems a view lists.
pub const MAX_PROBLEMS: usize = 8;

/// What the window asks, with a number the answer carries. The window asks
/// one thing at a time: it sends the next request once the view answers
/// this one, since the view holds only the answer to the last request.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub ask: Ask,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Ask {
    /// Check these settings, write them to the config file and use them.
    Save {
        settings: Draft,
        /// Replace a file that changed since the daemon read it, or that
        /// could not be used: the user agreed.
        replace: bool,
        /// Hotkeys without a modifier are what the user wants: they agreed.
        without_modifier: bool,
    },
}

/// Everything the window shows.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowView {
    /// The settings in the config file: what the daemon last read or saved.
    pub settings: Draft,
    /// The routes the daemon runs on; a change to `[input]` applies from its
    /// next start.
    pub running: InputDraft,
    pub activity: Activity,
    /// What each hotkey is now, as the tray names it.
    pub hotkeys: ShownHotkeys,
    /// Why no recording can start, whatever the activity; `None` when one
    /// can.
    pub no_recording: Option<NoRecording>,
    /// What is wrong now, as the tray lists it.
    pub problems: Vec<String>,
    /// The answer to the window's last request, once there is one.
    pub answer: Option<Answer>,
}

/// The hotkeys as they are now: the config's bindings on the kernel route,
/// the desktop's triggers on the portal route.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShownHotkeys {
    pub toggle_clicking: Hotkey,
    pub toggle_recording: Hotkey,
    pub toggle_playback: Hotkey,
}

/// The answer to request `id`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub id: u64,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    /// Written and in use.
    Saved,
    /// Written and in use, but not flushed to disk: why.
    SavedUnsynced(String),
    /// Nothing was written: the program is clicking, recording or playing,
    /// or about to. Save once it is idle.
    Busy,
    /// A value is not valid; nothing was written.
    Refused(Refusal),
    /// Nothing was written: the save needs the user's word first. Ask
    /// again with the matching flag set.
    Confirm(Confirm),
    /// Writing failed, or there is nowhere to write; nothing was written.
    Failed(String),
}

/// What a save needs the user's word on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Confirm {
    /// The file changed since it was read, or could not be used.
    Replace(Stale),
    /// This action's hotkey has no modifier, so it fires on a plain key
    /// press, in a game too.
    WithoutModifier(Action),
}

impl Outcome {
    /// The outcome with its message cut to [`MAX_TEXT`].
    pub fn bounded(self) -> Outcome {
        match self {
            Outcome::Refused(Refusal { field, reason }) => {
                Outcome::Refused(Refusal::new(field, bounded(&reason)))
            }
            Outcome::Failed(why) => Outcome::Failed(bounded(&why)),
            Outcome::SavedUnsynced(why) => Outcome::SavedUnsynced(bounded(&why)),
            other => other,
        }
    }
}

/// `text` cut to [`MAX_TEXT`] characters, with "…" where it was cut.
pub fn bounded(text: &str) -> String {
    match text.char_indices().nth(MAX_TEXT) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

/// A line that is not a message of the protocol.
#[derive(Debug)]
pub struct BadLine(String);

impl fmt::Display for BadLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BadLine {}

/// Reads one line, without its newline, as a `T`.
pub fn parse<T: DeserializeOwned>(line: &[u8]) -> Result<T, BadLine> {
    if line.len() >= MAX_LINE {
        return Err(BadLine(format!("a line is longer than {MAX_LINE} bytes")));
    }
    serde_json::from_slice(line).map_err(|err| BadLine(err.to_string()))
}

/// `message` as one line, newline included.
pub fn line(message: &impl Serialize) -> Result<Vec<u8>, BadLine> {
    let mut bytes = serde_json::to_vec(message).map_err(|err| BadLine(err.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() > MAX_LINE {
        return Err(BadLine(format!("a line is longer than {MAX_LINE} bytes")));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
