//! The clicking run: actions on absolute deadlines until the run finishes,
//! is stopped or the program shuts down.

use std::future::{Future, pending};
use std::pin::{Pin, pin};
use std::time::Duration;

use tokio::time::{Instant, sleep_until};

use super::timing::{self, Shape};
use super::{ClickerSettings, Mode, Position, Repeat};
use crate::input::{Held, Input, InputBackend, InputError};

/// Why a run ended. Whatever the reason, nothing it pressed is still held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// It made the configured number of actions.
    Finished,
    /// The stop signal fired.
    Stopped,
    /// The shutdown signal fired.
    ShutDown,
}

/// The two signals that end a run early. Each is a future that completes when
/// its signal fires; the controller decides what they wait on.
pub struct Signals<S, D> {
    /// The user stopped clicking.
    pub stop: S,
    /// The program is shutting down.
    pub shutdown: D,
}

/// How a run ended, and the input back when it is safe to reuse.
#[derive(Debug)]
pub struct RunEnd<B: InputBackend> {
    /// Why the run ended, or the first write error.
    pub result: Result<Ended, InputError>,
    /// The input, with nothing held. `None` when a release failed: the run
    /// then dropped the input, which closes its devices, and the kernel
    /// releases what they held. The caller opens the backend again.
    pub input: Option<Input<B>>,
}

/// Runs the clicker with `settings` until it finishes, a signal fires, or a
/// write fails.
///
/// The first action starts at once; each next one starts an interval after
/// the previous one's start, on absolute deadlines, so delays do not add up.
/// A run that falls behind continues from the present rather than catching up
/// with a burst. Every wait, including the hold inside a click, ends at once
/// when a signal fires, and a signal that has already fired prevents the
/// first action. Shutdown wins over a stop at the same moment.
///
/// Nothing stays pressed on any exit. The run releases everything held on
/// `input` and hands it back; if a release fails, it drops `input` instead,
/// closing the devices. Dropping this future part way drops `input` too, so
/// stop a run with its signal rather than by dropping it. A write error ends
/// the run and is returned; so is a failed release when the run had ended
/// without error.
pub async fn run<B, S, D>(
    mut input: Input<B>,
    settings: &ClickerSettings,
    rng: fastrand::Rng,
    signals: Signals<S, D>,
) -> RunEnd<B>
where
    B: InputBackend,
    S: Future<Output = ()>,
    D: Future<Output = ()>,
{
    let mut session = Session {
        input: &mut input,
        settings,
        rng,
        stop: pin!(signals.stop),
        shutdown: pin!(signals.shutdown),
    };
    let outcome = session.drive().await;
    let released = input.release_all();
    let clean = released.is_ok();
    RunEnd {
        result: outcome.and_then(|ended| released.map(|()| ended)),
        input: clean.then_some(input),
    }
}

/// One run's state. It borrows the input that `run` owns, so whatever ends
/// the run, `run` or the input's own drop releases what is held.
struct Session<'a, B: InputBackend, S, D> {
    input: &'a mut Input<B>,
    settings: &'a ClickerSettings,
    rng: fastrand::Rng,
    stop: Pin<&'a mut S>,
    shutdown: Pin<&'a mut D>,
}

/// The deadline after the action that was due at `due` and started at
/// `started`: an interval after `due`, which keeps delays from adding up. If
/// that has already passed by `now`, the run fell behind; it continues an
/// interval after the late action started, or at `now` if even that has
/// passed, never with a burst of catch-up actions. `None` when the deadline is
/// too far away to represent.
fn next_deadline(
    (due, started): (Instant, Instant),
    interval: Duration,
    now: Instant,
) -> Option<Instant> {
    let on_schedule = due.checked_add(interval)?;
    if on_schedule >= now {
        return Some(on_schedule);
    }
    started
        .checked_add(interval)
        .map(|rebased| rebased.max(now))
}

/// `Some(reason)` when a signal ended a wait, `None` when the wait ran out.
type Interrupted = Option<Ended>;

impl<B, S, D> Session<'_, B, S, D>
where
    B: InputBackend,
    S: Future<Output = ()>,
    D: Future<Output = ()>,
{
    async fn drive(&mut self) -> Result<Ended, InputError> {
        let settings = *self.settings;
        match settings.mode {
            Mode::HoldButton => self.hold(Held::Button(settings.button)).await,
            Mode::HoldKey => self.hold(Held::Key(settings.key)).await,
            Mode::Click | Mode::DoubleClick => self.repeat(Held::Button(settings.button)).await,
            Mode::KeyRepeat => self.repeat(Held::Key(settings.key)).await,
        }
    }

    /// Presses `item` once and holds it until a signal.
    async fn hold(&mut self, item: Held) -> Result<Ended, InputError> {
        if let Some(ended) = self.wait_until(Some(Instant::now())).await {
            return Ok(ended);
        }
        self.aim(item)?;
        self.press(item)?;
        Ok(self.until_signal().await)
    }

    /// One action per interval, until the repeat count or a signal.
    async fn repeat(&mut self, item: Held) -> Result<Ended, InputError> {
        let mut deadline = Some(Instant::now());
        let mut actions: u32 = 0;
        loop {
            if let Some(ended) = self.wait_until(deadline).await {
                return Ok(ended);
            }
            let started = Instant::now();
            let interval = timing::next_interval(self.settings, &mut self.rng);
            let shape = timing::shape(self.settings.mode, interval);
            if let Some(ended) = self.action(item, shape).await? {
                return Ok(ended);
            }
            actions = actions.saturating_add(1);
            if let Repeat::Times(count) = self.settings.repeat
                && actions >= count.get()
            {
                return Ok(Ended::Finished);
            }
            deadline =
                deadline.and_then(|due| next_deadline((due, started), interval, Instant::now()));
        }
    }

    /// One click, double click or key tap.
    async fn action(&mut self, item: Held, shape: Shape) -> Result<Interrupted, InputError> {
        self.aim(item)?;
        let first = Instant::now();
        if let Some(ended) = self.tap(item, shape).await? {
            return Ok(Some(ended));
        }
        let Some(gap) = shape.second_press else {
            return Ok(None);
        };
        if let Some(ended) = self.wait_until(first.checked_add(gap)).await {
            return Ok(Some(ended));
        }
        self.tap(item, shape).await
    }

    /// Presses `item`, holds it for the shape's hold and releases it.
    async fn tap(&mut self, item: Held, shape: Shape) -> Result<Interrupted, InputError> {
        self.press(item)?;
        if let Some(ended) = self
            .wait_until(Instant::now().checked_add(shape.hold))
            .await
        {
            return Ok(Some(ended));
        }
        self.release(item)?;
        Ok(None)
    }

    /// Moves to the fixed point before a button action. Keys go wherever the
    /// focus is, so they never move the pointer.
    fn aim(&mut self, item: Held) -> Result<(), InputError> {
        match (item, self.settings.position) {
            (Held::Button(_), Position::Fixed(point)) => self.input.move_absolute(point),
            _ => Ok(()),
        }
    }

    fn press(&mut self, item: Held) -> Result<(), InputError> {
        match item {
            Held::Button(button) => self.input.press_button(button),
            Held::Key(key) => self.input.press_key(key),
        }
    }

    fn release(&mut self, item: Held) -> Result<(), InputError> {
        match item {
            Held::Button(button) => self.input.release_button(button),
            Held::Key(key) => self.input.release_key(key),
        }
    }

    /// Waits until `deadline`, or forever when there is none (a deadline too
    /// far away to represent). Shutdown wins over stop, and both over a
    /// deadline that is due at the same moment.
    async fn wait_until(&mut self, deadline: Option<Instant>) -> Interrupted {
        let due = async {
            match deadline {
                Some(deadline) => sleep_until(deadline).await,
                None => pending().await,
            }
        };
        tokio::select! {
            biased;
            () = &mut self.shutdown => Some(Ended::ShutDown),
            () = &mut self.stop => Some(Ended::Stopped),
            () = due => None,
        }
    }

    /// Waits for stop or shutdown.
    async fn until_signal(&mut self) -> Ended {
        tokio::select! {
            biased;
            () = &mut self.shutdown => Ended::ShutDown,
            () = &mut self.stop => Ended::Stopped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_deadline_is_an_interval_later_or_now_when_behind() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let interval = Duration::from_millis(100);
        let deadlines = [
            // On time: an interval after the deadline, however late it woke.
            next_deadline((at(0), at(1)), interval, at(11)),
            // Due at 100, started at 350: the next is at 450, not at once.
            next_deadline((at(100), at(350)), interval, at(360)),
            // Even the rebased deadline has passed: now.
            next_deadline((at(100), at(350)), interval, at(500)),
        ];
        assert_eq!(deadlines, [Some(at(100)), Some(at(450)), Some(at(500))]);
    }
}
