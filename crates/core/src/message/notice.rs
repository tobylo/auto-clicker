//! What last happened that the user should know: why a start was refused
//! or cancelled, why a run stopped, how a recording was saved.

use std::fmt;

use super::{InputFailure, Quoted};
use crate::controller::{Blocked, NoRecording};
use crate::hotkeys::{Binding, Refusal, Unavailable};
use crate::playback::Fallback;
use crate::recording::{LoadError, MAX_FILE_BYTES};

/// The tray's notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// A start did not happen.
    Refused { start: Start, why: Hindrance },
    /// This recording cannot be played.
    Unplayable { recording: String, why: Unplayable },
    /// The playback hotkey found no recording.
    NothingToPlay,
    /// A run, a start or a recording stopped.
    Stopped(Stop),
    /// The devices failed under a run.
    RunFailed { playback: bool, why: InputFailure },
    /// A run did all it was asked: the recording it played, if it played
    /// one.
    Finished(Option<String>),
    /// A playback began, but not quite as asked.
    Playing(PlayStart),
    /// A recording ended, and why when it did not end by the user's hand.
    Recorded { stop: Option<Stop>, saving: Saving },
}

impl fmt::Display for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Notice::Refused { start, why } => write!(f, "{}: {why}", start.refused()),
            Notice::Unplayable { recording, why } => write!(f, "Not playing {recording}: {why}"),
            Notice::NothingToPlay => f.write_str("Nothing to play: no recordings yet"),
            Notice::Stopped(stop) => stop.fmt(f),
            Notice::RunFailed { playback, why } => {
                let what = if *playback { "Playback" } else { "Clicking" };
                write!(f, "{what} stopped: {why}")
            }
            Notice::Finished(Some(recording)) => write!(f, "Finished playing {recording}"),
            Notice::Finished(None) => f.write_str("Finished every action"),
            Notice::Playing(start) => start.fmt(f),
            Notice::Recorded {
                stop: Some(stop),
                saving,
            } => write!(f, "{stop}. {saving}"),
            Notice::Recorded { stop: None, saving } => saving.fmt(f),
        }
    }
}

/// What did not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Clicking,
    Playback,
    Recording,
}

impl Start {
    fn refused(self) -> &'static str {
        match self {
            Start::Clicking => "Not started",
            Start::Playback => "Not playing",
            Start::Recording => "Not recording",
        }
    }
}

/// Why a start did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hindrance {
    Blocked(Blocked),
    /// The run would press a key the desktop's hotkeys use.
    OwnKey(Refusal),
    /// The devices could not be opened.
    Input(InputFailure),
    /// The hotkey's modifiers stayed down for this many seconds.
    HeldTooLong {
        seconds: u64,
    },
    /// Whether the hotkey came up can no longer be known: press it again.
    ReleaseLost(Binding),
    HotkeysStopped,
    /// Clicking runs or is about to.
    ClickingOn,
    /// The recordings could not be listed.
    Unlisted(Quoted),
    NoRecording(NoRecording),
    NoClock,
    /// The keyboards can no longer be read.
    Hotkeys(Unavailable),
}

impl fmt::Display for Hindrance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Hindrance::Blocked(why) => why.fmt(f),
            Hindrance::OwnKey(why) => why.fmt(f),
            Hindrance::Input(why) => why.fmt(f),
            Hindrance::HeldTooLong { seconds } => write!(
                f,
                "the hotkey was held for {seconds} s; let go of it to start"
            ),
            Hindrance::ReleaseLost(hotkey) => write!(
                f,
                "could not tell when the hotkey was let go; press {hotkey} again"
            ),
            Hindrance::HotkeysStopped => f.write_str("hotkeys stopped working"),
            Hindrance::ClickingOn => f.write_str("clicking is on; stop it first"),
            Hindrance::Unlisted(why) => write!(f, "the recordings cannot be listed: {why}"),
            Hindrance::NoRecording(why) => why.fmt(f),
            Hindrance::NoClock => f.write_str("the clock could not be read"),
            Hindrance::Hotkeys(why) => why.fmt(f),
        }
    }
}

/// Why a recording cannot be played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unplayable {
    Missing,
    Unreadable(Quoted),
    TooLarge {
        mib: u64,
    },
    /// Not a valid recording: the reader's words.
    Invalid(Quoted),
    /// Shorter than this many milliseconds, and asked to play more than
    /// once.
    TooShort {
        ms: u128,
    },
    /// It presses a key the desktop's hotkeys use.
    PressesHotkeyKey(Refusal),
}

impl From<&LoadError> for Unplayable {
    fn from(err: &LoadError) -> Unplayable {
        match err {
            LoadError::Missing => Unplayable::Missing,
            LoadError::Unreadable(why) => Unplayable::Unreadable(Quoted::new(why.to_string())),
            LoadError::TooLarge => Unplayable::TooLarge {
                mib: MAX_FILE_BYTES / (1024 * 1024),
            },
            LoadError::Invalid(why) => Unplayable::Invalid(Quoted::new(why.to_string())),
        }
    }
}

impl fmt::Display for Unplayable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unplayable::Missing => f.write_str("the file is no longer there"),
            Unplayable::Unreadable(why) => write!(f, "the file could not be read: {why}"),
            Unplayable::TooLarge { mib } => write!(f, "the file is larger than {mib} MiB"),
            Unplayable::Invalid(why) => why.fmt(f),
            Unplayable::TooShort { ms } => write!(
                f,
                "it is shorter than {ms} ms, too short to play more than once"
            ),
            Unplayable::PressesHotkeyKey(why) => write!(f, "it presses a hotkey's key: {why}"),
        }
    }
}

/// Why a run, a start or a recording stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    Blocked(Blocked),
    /// It presses a key the desktop's hotkeys use now.
    OwnKey(Refusal),
    LayoutChanged,
    LayoutUnknown,
    /// The keyboards a recording reads can no longer be read.
    Hotkeys(Unavailable),
    /// The recording reached the largest size a recording may have.
    Full,
    /// The recording reached its time limit, in minutes.
    Limit {
        minutes: u64,
    },
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stop::Blocked(why) => write!(f, "Stopped: {why}"),
            Stop::OwnKey(why) => write!(f, "Stopped: {why}"),
            Stop::LayoutChanged => f.write_str("Playback stopped: the screen layout changed"),
            Stop::LayoutUnknown => {
                f.write_str("Playback stopped: the screen layout is no longer known")
            }
            Stop::Hotkeys(why) => write!(f, "Recording stopped: {why}"),
            Stop::Full => {
                f.write_str("Recording stopped: it reached the largest size a recording may have")
            }
            Stop::Limit { minutes } => {
                write!(f, "Recording stopped at its {minutes}-minute limit")
            }
        }
    }
}

/// How a playback began other than as asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayStart {
    /// The hotkey's modifiers were still down after this many seconds.
    Held { seconds: u64 },
    /// The recorded positions are not used.
    Relative(Fallback),
    /// Both.
    HeldAndRelative { seconds: u64, fallback: Fallback },
}

impl PlayStart {
    /// What to say, if anything: `held` is the wait the modifiers outlasted.
    pub fn new(held: Option<u64>, fallback: Option<Fallback>) -> Option<PlayStart> {
        match (held, fallback) {
            (Some(seconds), Some(fallback)) => {
                Some(PlayStart::HeldAndRelative { seconds, fallback })
            }
            (Some(seconds), None) => Some(PlayStart::Held { seconds }),
            (None, Some(fallback)) => Some(PlayStart::Relative(fallback)),
            (None, None) => None,
        }
    }
}

impl fmt::Display for PlayStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let held = |f: &mut fmt::Formatter<'_>, seconds: u64| {
            write!(
                f,
                "Playing although the hotkey was still held after {seconds} s; its keys mix \
                 into what plays"
            )
        };
        match self {
            PlayStart::Held { seconds } => held(f, *seconds),
            PlayStart::Relative(fallback) => fallback.fmt(f),
            PlayStart::HeldAndRelative { seconds, fallback } => {
                held(f, *seconds)?;
                write!(f, ". {fallback}")
            }
        }
    }
}

/// How saving a recording went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Saving {
    /// Nothing was recorded.
    Nothing,
    /// Saved under this name.
    Saved(String),
    Failed(SaveFailure),
}

impl fmt::Display for Saving {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Saving::Nothing => f.write_str("Nothing was recorded, so nothing was saved"),
            Saving::Saved(name) => write!(f, "Saved \u{201c}{name}\u{201d}"),
            Saving::Failed(why) => write!(f, "The recording could not be saved: {why}"),
        }
    }
}

/// Why a recording could not be saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveFailure {
    /// Neither `XDG_DATA_HOME` nor `HOME` is set.
    NoDataDirectory,
    /// What was recorded is not a valid recording: the check's words.
    Invalid(Quoted),
    /// Writing it failed.
    Write(Quoted),
}

impl fmt::Display for SaveFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveFailure::NoDataDirectory => {
                f.write_str("no data directory: neither XDG_DATA_HOME nor HOME is set")
            }
            SaveFailure::Invalid(why) | SaveFailure::Write(why) => why.fmt(f),
        }
    }
}
