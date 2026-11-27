//! Sending owed releases again (M5 brief decisions 6 and 7, WN06b): a
//! release that may not have arrived is sent again once it can arrive.
//!
//! [`Resend`] is the pass, kept apart from what wakes it so it can be
//! tested with a fake output, a scripted context and a given clock. Each
//! pass reads decision 7's context first and sends nothing it could not
//! deliver; what it sends, it judges as the run's own releases are judged.
//! A release Windows blocked waits for an event (the window in front or the
//! desktop changing, an unlock, a reconnect); one the context allowed but
//! Windows took nothing of (refused) is also tried on a timer, at most
//! [`TIMED_TRIES`] times, 100 ms after the refusal and doubling, and then on
//! events only. The tries are counted per item for as long as it is owed,
//! so events never give an item more.
//!
//! [`wait`] is where a pass sleeps between wakes.

mod wait;

use std::time::{Duration, Instant};

use auto_clicker_core::input::{Held, InputBackend, PressState};

use crate::delivery::{Delivery, judge};
use crate::watched::Probe;

#[cfg(test)]
pub(crate) use wait::take_signals_here;
pub use wait::{Also, Ended, Stop, send_until_delivered};
pub(crate) use wait::{stop_here, wake_here};

/// How many timed tries a refused release gets.
pub const TIMED_TRIES: u32 = 5;
/// The wait before the first timed try; each next one waits twice as long.
const FIRST_TRY: Duration = Duration::from_millis(100);

/// What woke a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// Something that may let a release arrive now: the first pass, a
    /// change of the window in front or of the desktop, an unlock, a
    /// reconnect. Every owed release is tried.
    Event,
    /// A timed try's deadline: only the releases due are tried.
    Timer,
}

/// When the next pass is due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Nothing is owed any more.
    Done,
    /// Something is owed, and only an event can deliver it.
    Events,
    /// Something is owed; a timed try is due at this deadline, or an
    /// event before it.
    Until(Instant),
}

/// The releases still owed, each with its timed tries.
#[derive(Debug)]
pub struct Resend {
    owed: Vec<Owing>,
}

/// One owed release.
#[derive(Clone, Copy, Debug)]
struct Owing {
    item: Held,
    /// The timed tries used so far.
    tried: u32,
    /// When its next timed try is due; set only after a refusal.
    due: Option<Instant>,
}

impl Resend {
    /// Owes each of `items`'s releases, none tried on a timer yet.
    pub fn new(items: impl IntoIterator<Item = Held>) -> Resend {
        let owed = items
            .into_iter()
            .map(|item| Owing {
                item,
                tried: 0,
                due: None,
            })
            .collect();
        Resend { owed }
    }

    /// How many releases are still owed.
    pub fn owed(&self) -> usize {
        self.owed.len()
    }

    /// Sends what `cause` wakes through `via`, at `now`; keeps what may not
    /// have arrived, and says when the next pass is due.
    pub fn pass<O: InputBackend, P: Probe>(
        &mut self,
        cause: Cause,
        now: Instant,
        via: &mut Via<O, P>,
    ) -> Next {
        let mut before = via.probe.context();
        let mut kept = Vec::with_capacity(self.owed.len());
        for mut owing in std::mem::take(&mut self.owed) {
            let timed = owing.due.is_some_and(|due| due <= now);
            let woken = cause == Cause::Event || timed;
            if timed {
                owing.tried = owing.tried.saturating_add(1);
                owing.due = None;
            }
            // Nothing is sent that could not arrive; it waits for an event.
            if !woken || !before.allows_delivery() {
                kept.push(owing);
                continue;
            }
            let sent = via.send_up(owing.item);
            let after = via.probe.context();
            match judge(before, after, &sent) {
                Delivery::Delivered => {}
                Delivery::Blocked => kept.push(owing),
                Delivery::Refused => {
                    owing.due = owing.due.or_else(|| next_try(owing.tried, now));
                    kept.push(owing);
                }
            }
            before = after;
        }
        self.owed = kept;
        self.next()
    }

    /// When the next pass is due.
    fn next(&self) -> Next {
        if self.owed.is_empty() {
            return Next::Done;
        }
        self.owed
            .iter()
            .filter_map(|owing| owing.due)
            .min()
            .map_or(Next::Events, Next::Until)
    }
}

/// When the timed try after `tried` of them is due, from a refusal at
/// `now`: none once all are used.
fn next_try(tried: u32, now: Instant) -> Option<Instant> {
    if tried >= TIMED_TRIES {
        return None;
    }
    let wait = FIRST_TRY.checked_mul(2u32.checked_pow(tried)?)?;
    now.checked_add(wait)
}

/// What a pass sends through, and what says what is in front.
#[derive(Debug)]
pub struct Via<O, P> {
    pub output: O,
    pub probe: P,
}

impl<O: InputBackend, P> Via<O, P> {
    /// Sends `item`'s release.
    fn send_up(&mut self, item: Held) -> Result<(), auto_clicker_core::input::InputError> {
        match item {
            Held::Button(button) => self.output.button(button, PressState::Up),
            Held::Key(key) => self.output.key(key, PressState::Up),
        }
    }
}

#[cfg(test)]
mod tests;
