//! The clicking run: actions on absolute deadlines until the run finishes,
//! is stopped or the program shuts down.

use std::future::{Future, pending};
use std::pin::{Pin, pin};

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
/// On every exit, including this future being dropped part way, the run
/// releases everything held on `input`. A write error ends the run and is
/// returned; so is a failed release when the run had ended without error.
pub async fn run<B, S, D>(
    input: &mut Input<B>,
    settings: &ClickerSettings,
    rng: fastrand::Rng,
    signals: Signals<S, D>,
) -> Result<Ended, InputError>
where
    B: InputBackend,
    S: Future<Output = ()>,
    D: Future<Output = ()>,
{
    let mut session = Session {
        input,
        settings,
        rng,
        stop: pin!(signals.stop),
        shutdown: pin!(signals.shutdown),
    };
    let outcome = session.drive().await;
    let released = session.input.release_all();
    outcome.and_then(|ended| released.map(|()| ended))
}

/// One run's state. Dropping it releases everything held, which covers a run
/// whose future is dropped before it returns.
struct Session<'a, B: InputBackend, S, D> {
    input: &'a mut Input<B>,
    settings: &'a ClickerSettings,
    rng: fastrand::Rng,
    stop: Pin<&'a mut S>,
    shutdown: Pin<&'a mut D>,
}

impl<B: InputBackend, S, D> Drop for Session<'_, B, S, D> {
    /// Errors cannot be reported from here; [`run`] reports them on its own
    /// exits, and the backend's devices release the rest when they close.
    fn drop(&mut self) {
        let _ = self.input.release_all();
    }
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
            // A deadline already past fires now, and the next one counts from
            // there: a run that fell behind does not catch up with a burst.
            deadline = deadline
                .and_then(|due| due.checked_add(interval))
                .map(|next| next.max(Instant::now()));
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
