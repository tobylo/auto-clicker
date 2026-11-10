//! Recordings: what the user did with the mouse and keyboard, with its
//! timing, as one file each.
//!
//! A [`Recording`] is only ever built through [`Recording::new`], which checks
//! it, so every recording in memory is one playback can trust: times never go
//! back, every release follows its press, and nothing is down at the end.
//! Files are JSON ([`json`]); the [`Recorder`] builds a recording from what
//! the input reader sees, and [`store`] saves it.
//!
//! Key data stays inside a recording: [`Recording`]'s `Debug` shows how many
//! events it holds, not which, and no error here names a key.

mod codes;
mod created;
mod json;
mod recorder;
mod store;
mod tap;
mod trim;

use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

pub use codes::{name as code_name, parse as parse_code, recordable};
pub use created::{Created, LocalTime};
pub use recorder::{Ending, KeyInput, Recorder};
pub use store::{recordings_dir, save};
pub use tap::{RecordingFeed, RecordingSlot};

use crate::input::Held;

/// The format version this program writes and reads.
pub const VERSION: u64 = 1;

/// The longest recording: a recording stops and saves when it reaches this.
pub const MAX_DURATION: Duration = Duration::from_secs(60 * 60);

/// [`MAX_DURATION`] in microseconds, the unit of a recording's times.
const MAX_DURATION_US: u64 = 60 * 60 * 1_000_000;

/// The longest name a recording may have, in characters.
const MAX_NAME_CHARS: usize = 200;

/// What happened at one moment of a recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Press(Held),
    Release(Held),
    /// Relative pointer motion, summed over up to a millisecond.
    Move {
        dx: i32,
        dy: i32,
    },
    /// Whole wheel notches, summed over up to a millisecond; positive
    /// scrolls up.
    Wheel(i32),
}

/// One change, `at` microseconds after the recording started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub at: u64,
    pub change: Change,
}

/// A checked recording.
#[derive(Clone, PartialEq, Eq)]
pub struct Recording {
    name: String,
    created: Created,
    duration_us: u64,
    events: Vec<Event>,
}

/// Why something is not a valid recording. The reason never names a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidRecording(String);

impl fmt::Display for InvalidRecording {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidRecording {}

impl InvalidRecording {
    fn new(reason: impl Into<String>) -> InvalidRecording {
        InvalidRecording(reason.into())
    }

    fn at(index: usize, reason: &str) -> InvalidRecording {
        InvalidRecording(format!("events[{index}] {reason}"))
    }
}

impl Recording {
    /// Checks the parts of a recording and puts them together.
    ///
    /// # Errors
    /// The name is empty, longer than 200 characters or holds a control
    /// character; the duration is over [`MAX_DURATION`]; an event is later
    /// than the duration or earlier than the one before it; a key or button
    /// has no name ([`recordable`]); a press finds its key already down, a
    /// release finds it up, or something is still down at the end; a move
    /// or wheel event changes nothing.
    pub fn new(
        name: String,
        created: Created,
        duration_us: u64,
        events: Vec<Event>,
    ) -> Result<Recording, InvalidRecording> {
        check_name(&name)?;
        if duration_us > MAX_DURATION_US {
            return Err(InvalidRecording::new(
                "the recording is longer than 60 minutes",
            ));
        }
        check_events(&events, duration_us)?;
        Ok(Recording {
            name,
            created,
            duration_us,
            events,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn created(&self) -> Created {
        self.created
    }

    /// From the start to the end of the recording, idle time at the end
    /// included.
    pub fn duration(&self) -> Duration {
        Duration::from_micros(self.duration_us)
    }

    pub fn duration_us(&self) -> u64 {
        self.duration_us
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }
}

impl fmt::Debug for Recording {
    /// Leaves the events out: they are what the user typed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recording")
            .field("name", &self.name)
            .field("created", &self.created)
            .field("duration_us", &self.duration_us)
            .field("events", &self.events.len())
            .finish()
    }
}

fn check_name(name: &str) -> Result<(), InvalidRecording> {
    if name.trim().is_empty() {
        return Err(InvalidRecording::new("the recording has no name"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(InvalidRecording::new(format!(
            "the recording's name is longer than {MAX_NAME_CHARS} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(InvalidRecording::new(
            "the recording's name holds a control character",
        ));
    }
    Ok(())
}

fn check_events(events: &[Event], duration_us: u64) -> Result<(), InvalidRecording> {
    let mut down = BTreeSet::new();
    let mut last = 0;
    for (index, event) in events.iter().enumerate() {
        if event.at < last {
            return Err(InvalidRecording::at(
                index,
                "is earlier than the event before it",
            ));
        }
        if event.at > duration_us {
            return Err(InvalidRecording::at(
                index,
                "is later than the end of the recording",
            ));
        }
        last = event.at;
        check_change(event.change, &mut down).map_err(|why| InvalidRecording::at(index, why))?;
    }
    if down.is_empty() {
        Ok(())
    } else {
        Err(InvalidRecording::new(
            "the recording ends with a key or button still down",
        ))
    }
}

fn check_change(change: Change, down: &mut BTreeSet<Held>) -> Result<(), &'static str> {
    let named = |held| {
        codes::name(held)
            .map(|_| held)
            .ok_or("names no key or button")
    };
    let fine = match change {
        Change::Press(held) => down.insert(named(held)?),
        Change::Release(held) => down.remove(&named(held)?),
        Change::Move { dx, dy } => dx != 0 || dy != 0,
        Change::Wheel(notches) => notches != 0,
    };
    fine.then_some(()).ok_or(match change {
        Change::Press(_) => "presses a key or button that is already down",
        Change::Release(_) => "releases a key or button that is not down",
        Change::Move { .. } => "moves by nothing",
        Change::Wheel(_) => "turns the wheel by nothing",
    })
}

#[cfg(test)]
mod tests;
