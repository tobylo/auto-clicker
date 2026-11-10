//! The playback run: every event of a recording at its own absolute
//! deadline, once or in a loop, until it finishes, is stopped or the program
//! shuts down.

use std::future::Future;
use std::pin::pin;
use std::time::Duration;

use tokio::time::Instant;

use super::{Playback, Speed};
use crate::clicker::Repeat;
use crate::input::{Held, Input, InputBackend, InputError};
use crate::recording::Change;
use crate::run::{Ended, Interrupted, RunEnd, Signals, Waiter, finish};

/// How many events that are already due fire before the run hands the thread
/// back. The run's signals fire only while the controller runs, so a long
/// batch of events at one moment must not keep it from running.
const BATCH: u32 = 256;

/// Plays `playback` until it has played as often as its settings say, a
/// signal fires, or a write fails.
///
/// Each event fires at the run's start plus its time in the recording
/// divided by the speed; in a later round, the rounds before it are added
/// first, each as long as the whole recording. Every deadline is computed
/// from the start, so delays never add up. Events that are already due when
/// the run wakes fire in order without waiting, and a late event does not
/// move the ones after it.
///
/// Every wait ends at once when a signal fires, and a signal that has
/// already fired prevents the first event. Shutdown wins over a stop at the
/// same moment. Nothing stays pressed on any exit, exactly as for a clicking
/// run ([`crate::clicker::run`]): stop a run with its signal rather than by
/// dropping it.
pub async fn play<B, S, D>(
    mut input: Input<B>,
    playback: &Playback,
    signals: Signals<S, D>,
) -> RunEnd<B>
where
    B: InputBackend,
    S: Future<Output = ()>,
    D: Future<Output = ()>,
{
    let mut session = Session {
        input: &mut input,
        playback,
        waiter: Waiter {
            stop: pin!(signals.stop),
            shutdown: pin!(signals.shutdown),
        },
        due_in_a_row: 0,
    };
    let outcome = session.drive().await;
    finish(input, outcome)
}

/// When each event of a run is due.
struct Schedule {
    start: Instant,
    /// One round's length in the recording's own time, in microseconds.
    round_us: u64,
    speed: Speed,
}

impl Schedule {
    /// When the event `offset_us` into round `round` (from 0) is due. `None`
    /// when that is too far away to represent.
    fn at(&self, round: u64, offset_us: u64) -> Option<Instant> {
        let recorded_us = u128::from(round)
            .checked_mul(u128::from(self.round_us))?
            .checked_add(u128::from(offset_us))?;
        // Microseconds over thousandths: nanoseconds at the playing speed.
        let nanos = recorded_us.checked_mul(1_000_000)? / u128::from(self.speed.thousandths());
        let seconds = u64::try_from(nanos / 1_000_000_000).ok()?;
        let rest = u32::try_from(nanos % 1_000_000_000).ok()?;
        self.start.checked_add(Duration::new(seconds, rest))
    }
}

/// One run's state. It borrows the input that `play` owns, so whatever ends
/// the run, `play` or the input's own drop releases what is held.
struct Session<'a, B: InputBackend, S, D> {
    input: &'a mut Input<B>,
    playback: &'a Playback,
    waiter: Waiter<'a, S, D>,
    /// How many events in a row were already due when their turn came.
    due_in_a_row: u32,
}

impl<B, S, D> Session<'_, B, S, D>
where
    B: InputBackend,
    S: Future<Output = ()>,
    D: Future<Output = ()>,
{
    async fn drive(&mut self) -> Result<Ended, InputError> {
        let schedule = Schedule {
            start: Instant::now(),
            round_us: self.playback.recording.duration_us(),
            speed: self.playback.settings.speed,
        };
        let mut round: u64 = 0;
        loop {
            if let Some(ended) = self.round(&schedule, round).await? {
                return Ok(ended);
            }
            // A count that cannot go on counting has played long enough.
            let Some(played) = round.checked_add(1) else {
                return Ok(Ended::Finished);
            };
            if let Repeat::Times(count) = self.playback.settings.repeat
                && played >= u64::from(count.get())
            {
                return Ok(Ended::Finished);
            }
            round = played;
        }
    }

    /// Plays the recording once, as round `round`. The round begins with a
    /// wait for its own start, so an empty recording still takes its time.
    async fn round(&mut self, schedule: &Schedule, round: u64) -> Result<Interrupted, InputError> {
        if let Some(ended) = self.wait_for(schedule.at(round, 0)).await {
            return Ok(Some(ended));
        }
        let playback = self.playback;
        for event in playback.recording.events() {
            if let Some(ended) = self.wait_for(schedule.at(round, event.at)).await {
                return Ok(Some(ended));
            }
            self.apply(event.change)?;
        }
        Ok(None)
    }

    /// Waits until `deadline`. A deadline too far away to represent ends the
    /// run as finished. After [`BATCH`] deadlines in a row that had already
    /// passed, hands the thread back once.
    async fn wait_for(&mut self, deadline: Option<Instant>) -> Interrupted {
        let Some(deadline) = deadline else {
            return Some(Ended::Finished);
        };
        let waits = deadline > Instant::now();
        if let Some(ended) = self.waiter.wait_until(Some(deadline)).await {
            return Some(ended);
        }
        if waits {
            self.due_in_a_row = 0;
            return None;
        }
        self.due_in_a_row = self.due_in_a_row.saturating_add(1);
        if self.due_in_a_row < BATCH {
            return None;
        }
        self.due_in_a_row = 0;
        self.waiter.yield_once().await
    }

    fn apply(&mut self, change: Change) -> Result<(), InputError> {
        match change {
            Change::Press(Held::Button(button)) => self.input.press_button(button),
            Change::Press(Held::Key(key)) => self.input.press_key(key),
            Change::Release(Held::Button(button)) => self.input.release_button(button),
            Change::Release(Held::Key(key)) => self.input.release_key(key),
            Change::Move { dx, dy } => self.input.move_relative(dx, dy),
            Change::Wheel(notches) => self.input.scroll(notches),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule(round_us: u64, speed: f64) -> (Instant, Schedule) {
        let start = Instant::now();
        let speed = Speed::from_ratio(speed).unwrap();
        (
            start,
            Schedule {
                start,
                round_us,
                speed,
            },
        )
    }

    #[test]
    fn an_event_is_due_its_offset_over_the_speed_after_the_rounds_before_it() {
        let after = |round_us, speed, round, offset_us| {
            let (start, schedule) = schedule(round_us, speed);
            schedule.at(round, offset_us).map(|at| at - start)
        };
        let ns = Duration::from_nanos;
        assert_eq!(after(1_000_000, 1.0, 0, 1_337), Some(ns(1_337_000)));
        assert_eq!(after(1_000_000, 1.0, 3, 1_337), Some(ns(3_001_337_000)));
        assert_eq!(after(1_000_000, 4.0, 3, 1_000), Some(ns(750_250_000)));
        assert_eq!(after(1_000_000, 0.25, 1, 500_000), Some(ns(6_000_000_000)));
        // A third of a microsecond is kept, not rounded per event.
        assert_eq!(after(0, 3.0, 0, 1), Some(ns(333)));
    }

    #[test]
    fn a_deadline_too_far_away_is_none() {
        let (_, schedule) = schedule(3_600_000_000, 0.25);
        assert_eq!(schedule.at(u64::MAX, u64::MAX), None);
        assert!(schedule.at(u64::from(u32::MAX), 0).is_some());
    }
}
