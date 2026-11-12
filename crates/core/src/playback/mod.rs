//! Playing a recording back: its settings, the check a recording passes
//! before it may play, and the run that replays it.
//!
//! Playback is a run like clicking ([`crate::run`]): one async function,
//! [`play`], which the controller polls. It reaches devices only through
//! [`Input`](crate::input::Input), and ends at once on the stop or shutdown
//! signal it is given.
//!
//! A recording with positions plays them as absolute moves when the screens
//! they lie on are unchanged ([`Playback::motion`]), following its [`plan`];
//! otherwise it plays with relative motion, as one without positions does.

mod engine;
mod plan;
mod settings;

use std::num::NonZeroU32;
use std::time::Duration;

pub use engine::play;
pub use plan::{COVER_US, DELIVERY_DELAY_US};
pub use settings::{PlaybackSettings, Speed};

use crate::clicker::Repeat;
use crate::recording::{Event, Layout, Recording};
use plan::Plan;

/// The shortest recording that may play more than once. Shorter ones would
/// loop faster than a person could stop them.
pub const MIN_LOOP_LENGTH: Duration = Duration::from_millis(100);

/// A recording and how to play it, checked: this is what [`play`] takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playback {
    recording: Recording,
    settings: PlaybackSettings,
    /// How its positions play; `None` without positions. Boxed: most
    /// recordings have none.
    plan: Option<Box<Plan>>,
}

/// How a run moves the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Motion {
    /// Positions as absolute moves across this layout, the current one.
    Positions(Layout),
    /// Relative moves only; positions, if any, are not sent.
    Relative,
}

/// Why a recording's positions are not played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// The current layout is not known.
    Unknown,
    /// A screen a position lies on moved, changed size or is gone.
    Changed,
}

/// What the tray says while it plays.
impl std::fmt::Display for Fallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Fallback::Unknown => {
                "Playing with relative motion: the screen layout is not known, so the recorded positions cannot be used"
            }
            Fallback::Changed => {
                "Playing with relative motion: a screen this recording used has moved, changed size or is gone"
            }
        })
    }
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
                plan: Plan::new(&recording).map(Box::new),
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

    /// How it plays on `current`, the layout now (`None` when unknown), and
    /// why its positions are not used when it has some that are not.
    pub fn motion(&self, current: Option<&Layout>) -> (Motion, Option<Fallback>) {
        let Some(plan) = &self.plan else {
            return (Motion::Relative, None);
        };
        match current {
            None => (Motion::Relative, Some(Fallback::Unknown)),
            Some(layout) if plan.fits(layout) => (Motion::Positions(layout.clone()), None),
            Some(_) => (Motion::Relative, Some(Fallback::Changed)),
        }
    }

    /// The events a run with `motion` sends, each at its time.
    fn steps<'a>(&'a self, motion: &Motion) -> Box<dyn Iterator<Item = Event> + 'a> {
        let events = self.recording.events();
        match (motion, &self.plan) {
            (Motion::Positions(_), Some(plan)) => Box::new(plan.steps(events)),
            _ => Box::new(
                events
                    .iter()
                    .copied()
                    .filter(|event| !matches!(event.change, crate::recording::Change::Position(_))),
            ),
        }
    }
}

#[cfg(test)]
mod tests;
