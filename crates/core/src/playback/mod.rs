//! Playing a recording back: its settings, the check a recording passes
//! before it may play, and the run that replays it.
//!
//! Playback is a run like clicking ([`crate::run`]): one async function,
//! [`play`], which the controller polls. It reaches devices only through
//! [`Input`](crate::input::Input), and ends at once on the stop or shutdown
//! signal it is given.

mod engine;
mod settings;

use std::num::NonZeroU32;
use std::time::Duration;

pub use engine::play;
pub use settings::{PlaybackSettings, Speed};

use crate::clicker::Repeat;
use crate::recording::Recording;

/// The shortest recording that may play more than once. Shorter ones would
/// loop faster than a person could stop them.
pub const MIN_LOOP_LENGTH: Duration = Duration::from_millis(100);

/// A recording and how to play it, checked: this is what [`play`] takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playback {
    recording: Recording,
    settings: PlaybackSettings,
}

/// The recording is shorter than [`MIN_LOOP_LENGTH`] and was asked to play
/// more than once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooShortToLoop;

impl Playback {
    /// `recording` played as `settings` say.
    ///
    /// # Errors
    /// The settings repeat the recording (any count above one, or until
    /// stopped) and it is shorter than [`MIN_LOOP_LENGTH`], trailing idle
    /// time included.
    pub fn new(
        recording: Recording,
        settings: PlaybackSettings,
    ) -> Result<Playback, TooShortToLoop> {
        let once = settings.repeat == Repeat::Times(NonZeroU32::MIN);
        if once || recording.duration() >= MIN_LOOP_LENGTH {
            Ok(Playback {
                recording,
                settings,
            })
        } else {
            Err(TooShortToLoop)
        }
    }

    pub fn recording(&self) -> &Recording {
        &self.recording
    }
}

#[cfg(test)]
mod tests;
