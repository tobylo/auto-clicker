//! What the settings window and the daemon say to each other over the
//! socket the daemon gives the window: one JSON object per line, each at
//! most [`MAX_LINE`] bytes.
//!
//! The window sends [`Request`]s. The daemon sends [`Update`]s: a
//! [`WindowView`] at the start and after every change, everything the window shows, including the
//! answer to its last request. Answers are part of the view rather than
//! messages of their own, so a newer view never hides one, and the daemon
//! only ever has the latest view to send.
//!
//! Each side parses what it reads once, into these types; anything else
//! (an unknown field, a line too long) ends the connection.

use std::fmt;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::{Draft, InputDraft, Refusal, Stale};
use crate::controller::{Activity, Hotkey, Listed, NoRecording, Routes};
use crate::hotkeys::{Action, Binding, Capture};
use crate::message::{
    ChangeFailure, Failure, Notice, Problem, Quoted, RecordingChange, Unavailability,
};
use crate::recording::FileName;

/// The longest line either side sends or reads, newline included. A view
/// with the most recordings the window lists, each with the longest name,
/// fits with room to spare (`window::tests`).
pub const MAX_LINE: usize = 1024 * 1024;

/// The most characters of one text from outside the program (a reader's
/// error, a trigger the desktop reports, a parser's message) a view
/// carries; a longer one is cut, so the view still fits in [`MAX_LINE`],
/// escaped as JSON escapes it.
pub const MAX_TEXT: usize = 500;

/// The most problems a view lists.
pub const MAX_PROBLEMS: usize = 8;

/// What the daemon sends the window.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Update {
    /// Everything the window shows now.
    View(Box<WindowView>),
    /// Settings was chosen in the tray again: come to the front, if the
    /// desktop lets you.
    Raise,
}

/// What the window asks, with a number the answer carries. The window asks
/// one thing at a time: it sends the next request once the view answers
/// this one, since the view holds only the answer to the last request. A
/// request while a capture or the desktop's settings are pending is
/// answered [`Outcome::Busy`].
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
        /// The actions whose hotkey without a modifier the user agreed to,
        /// as a [`Confirm::WithoutModifier`] named them.
        without_modifier: Vec<Action>,
    },
    /// Read what the user presses next, for a hotkey or the clicker's key.
    /// Hotkeys do nothing until it ends: with a capture, Esc, after
    /// [`CAPTURE_TIME`], when the window goes, or when anything else
    /// happens to the program. The answer comes when it ends.
    Capture(Capture),
    /// Open the desktop's settings for the program's shortcuts, where the
    /// desktop delivers the hotkeys. The token, from the window's
    /// activation, lets the desktop raise the settings.
    ConfigureShortcuts { activation_token: Option<String> },
    /// Do `action` with the recording in `file`. Answered
    /// [`Outcome::Busy`] unless the program is idle.
    Recording {
        file: FileName,
        action: RecordingAction,
    },
}

/// What the window does with a recording.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordingAction {
    /// Give it this name, in its file.
    Rename { name: String },
    /// Delete its file, for good: the user agreed.
    Delete,
    /// Make it the one the playback hotkey plays.
    Choose,
    /// Play it after the tray's countdown; it becomes the current one.
    Play,
}

/// How long a capture waits for a key.
pub const CAPTURE_TIME: Duration = Duration::from_secs(10);

/// The longest activation token taken; the desktop's are far shorter.
pub const MAX_TOKEN: usize = 256;

/// Everything the window shows.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowView {
    /// The settings in the config file: what the daemon last read or saved.
    pub settings: Draft,
    /// The routes the config gave the daemon at its start, `auto` included;
    /// a change to `[input]` applies from its next start.
    pub running: InputDraft,
    /// The routes the daemon picked from those at its start.
    pub routes: Routes,
    pub activity: Activity,
    /// What each hotkey is now, as the tray names it.
    pub hotkeys: ShownHotkeys,
    /// Why no recording can start, whatever the activity; `None` when one
    /// can.
    pub no_recording: Option<NoRecording>,
    /// What is wrong now, as the tray lists it.
    pub problems: Vec<Problem>,
    pub recordings: RecordingsView,
    /// The answer to the window's last request, once there is one.
    pub answer: Option<Answer>,
}

/// The recordings, as the controller last listed them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingsView {
    /// The newest, newest first, at most
    /// [`WINDOW_RECORDINGS`](crate::controller::WINDOW_RECORDINGS).
    pub listed: Vec<Listed>,
    /// How many more there are than are listed.
    pub more: u64,
    /// The one the playback hotkey plays, when it is listed.
    pub current: Option<FileName>,
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
    SavedUnsynced(Quoted),
    /// Nothing was written: the program is clicking, recording or playing,
    /// or about to. Save once it is idle.
    Busy,
    /// A value is not valid; nothing was written.
    Refused(Refusal),
    /// Nothing was written: the save needs the user's word first. Ask
    /// again with the matching flag set.
    Confirm(Confirm),
    /// Writing failed, or there is nowhere to write; nothing was written.
    Failed(Failure),
    /// What a capture got: a hotkey, which may have no modifier, or the
    /// clicker's key, as a Linux key code.
    CapturedHotkey(Binding),
    CapturedKey(u16),
    /// A capture ended without a key.
    CaptureEnded(CaptureEnd),
    /// The desktop opened its settings for the program's shortcuts.
    Configured,
    /// This cannot be done here, and why: capturing where the desktop
    /// delivers the hotkeys or no keyboard can be read, or opening the
    /// desktop's settings where the program reads the keyboards.
    Unavailable(Unavailability),
    /// What came of a [`Ask::Recording`] that was not busy.
    Recording(RecordingOutcome),
}

/// What came of something done with a recording.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordingOutcome {
    Renamed,
    Deleted,
    Chosen,
    /// Its countdown began, or it plays.
    Playing,
    /// Done, but it may not survive a power loss: why.
    Unsynced(RecordingChange, Quoted),
    /// Nothing changed, and why.
    NotChanged(RecordingChange, ChangeFailure),
    /// It does not play: what the tray says about it, if anything.
    NotPlaying(Option<Notice>),
}

/// What the window says of it, in English.
impl fmt::Display for RecordingOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordingOutcome::Renamed => f.write_str("Renamed."),
            RecordingOutcome::Deleted => f.write_str("Deleted."),
            RecordingOutcome::Chosen => {
                f.write_str("The playback hotkey plays this recording now.")
            }
            RecordingOutcome::Playing => f.write_str("Playing."),
            RecordingOutcome::Unsynced(change, why) => write!(
                f,
                "{}, but it may not survive a power loss: {why}",
                change.done()
            ),
            RecordingOutcome::NotChanged(change, why) => write!(f, "{}: {why}", change.not_done()),
            RecordingOutcome::NotPlaying(Some(notice)) => notice.fmt(f),
            RecordingOutcome::NotPlaying(None) => f.write_str("Not playing"),
        }
    }
}

/// Why a capture ended without a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureEnd {
    /// Esc was pressed.
    Cancelled,
    /// Nothing was pressed within [`CAPTURE_TIME`].
    TimedOut,
    /// A key without a name was pressed, which no hotkey can use.
    Unnamed,
    /// Something else happened: a start, a stop, the session locked, the
    /// keyboards could no longer be read, or the program quit.
    Interrupted,
}

/// What a save needs the user's word on.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Confirm {
    /// The file changed since it was read, or could not be used.
    Replace(Stale),
    /// These actions' new hotkeys have no modifier, so they fire on a plain
    /// key press, in a game too. Each needs the user's word.
    WithoutModifier(Vec<Action>),
}

impl WindowView {
    /// The view with the triggers the desktop reports cut to [`MAX_TEXT`]
    /// and at most [`MAX_PROBLEMS`] problems, so it always fits a line.
    /// Text from outside the program in a message is [`Quoted`], which is
    /// cut on its way out.
    pub fn bounded(self) -> WindowView {
        let shown = |hotkey: Hotkey| match hotkey {
            Hotkey::Desktop(Some(trigger)) => Hotkey::Desktop(Some(bounded(&trigger))),
            other => other,
        };
        let ShownHotkeys {
            toggle_clicking,
            toggle_recording,
            toggle_playback,
        } = self.hotkeys;
        WindowView {
            hotkeys: ShownHotkeys {
                toggle_clicking: shown(toggle_clicking),
                toggle_recording: shown(toggle_recording),
                toggle_playback: shown(toggle_playback),
            },
            problems: self.problems.into_iter().take(MAX_PROBLEMS).collect(),
            ..self
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
