//! What every run shares, clicking or playing a recording: the signals that
//! end it, its one way of waiting, and how it lets go of everything at its
//! end.
//!
//! A run is one async function the controller polls. It reaches devices only
//! through [`Input`], waits only through its [`Waiter`], and ends through
//! [`finish`], so every run stops at once on a signal and leaves nothing
//! pressed.

use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::task::Poll;

use tokio::time::Instant;

use crate::input::{Input, InputBackend, InputError, RunTimer};

/// Why a run ended. Whatever the reason, nothing it pressed is still held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// It did everything it was configured to do.
    Finished,
    /// The stop signal fired.
    Stopped,
    /// The shutdown signal fired.
    ShutDown,
}

/// The two signals that end a run early. Each is a future that completes when
/// its signal fires; the controller decides what they wait on.
pub struct Signals<S, D> {
    /// The user stopped the run.
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

/// Ends a run: releases everything held on `input` and hands it back. If a
/// release fails, `input` is dropped instead, closing the devices, and the
/// failure is the run's result unless the run had already failed.
pub(crate) fn finish<B: InputBackend>(
    mut input: Input<B>,
    outcome: Result<Ended, InputError>,
) -> RunEnd<B> {
    let released = input.release_all();
    let clean = released.is_ok();
    RunEnd {
        result: outcome.and_then(|ended| released.map(|()| ended)),
        input: clean.then_some(input),
    }
}

/// A started run's timer and input, in the order they are bound.
pub(crate) type Started<B> = (Box<dyn RunTimer>, Input<B>);

/// Starts a run on `input`: makes its timer before the first deadline. A
/// run that cannot have one ends at once, pressing nothing. The timer comes
/// first in the pair, so bound as `let (timer, input)` the input drops
/// first: a run dropped part way releases what it holds before its timer
/// ends (on Windows, joins its thread).
pub(crate) fn with_timer<B: InputBackend>(mut input: Input<B>) -> Result<Started<B>, RunEnd<B>> {
    match input.run_timer() {
        Ok(timer) => Ok((timer, input)),
        Err(err) => Err(finish(input, Err(err))),
    }
}

/// `Some(reason)` when a signal ended a wait, `None` when the wait ran out.
pub(crate) type Interrupted = Option<Ended>;

/// A run's signals and its timer, and every wait a run makes. Shutdown wins
/// over a stop that fires at the same moment.
pub(crate) struct Waiter<'a, S, D> {
    pub(crate) stop: Pin<&'a mut S>,
    pub(crate) shutdown: Pin<&'a mut D>,
    /// The run's timer, from its backend: the only way it waits for a
    /// deadline.
    pub(crate) timer: &'a mut dyn RunTimer,
}

impl<S, D> Waiter<'_, S, D>
where
    S: Future<Output = ()>,
    D: Future<Output = ()>,
{
    /// Waits until `deadline`, or forever when there is none (a deadline too
    /// far away to represent). Both signals win over a deadline that is due
    /// at the same moment. Fails when the run's timer does, with or without
    /// a deadline.
    pub(crate) async fn wait_until(
        &mut self,
        deadline: Option<Instant>,
    ) -> Result<Interrupted, InputError> {
        let timer = &mut *self.timer;
        let due = poll_fn(|cx| match deadline {
            Some(deadline) => timer.poll_until(cx, deadline),
            None => timer.poll_failed(cx).map(Err),
        });
        tokio::select! {
            biased;
            () = &mut self.shutdown => Ok(Some(Ended::ShutDown)),
            () = &mut self.stop => Ok(Some(Ended::Stopped)),
            due = due => due.map(|()| None),
        }
    }

    /// Waits for stop or shutdown. Fails when the run's timer does, so a
    /// hold whose timer stopped working ends and lets go.
    pub(crate) async fn until_signal(&mut self) -> Result<Ended, InputError> {
        let timer = &mut *self.timer;
        let failed = poll_fn(|cx| timer.poll_failed(cx));
        tokio::select! {
            biased;
            () = &mut self.shutdown => Ok(Ended::ShutDown),
            () = &mut self.stop => Ok(Ended::Stopped),
            err = failed => Err(err),
        }
    }

    /// Hands the thread back once, so the controller can handle what is
    /// queued (a stop, say) before the run goes on. A run with much to do at
    /// one moment calls this between batches: its signals fire only while
    /// the controller runs.
    pub(crate) async fn yield_once(&mut self) -> Interrupted {
        let mut yielded = false;
        let resumed = poll_fn(move |cx| {
            if yielded {
                return Poll::Ready(());
            }
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        });
        tokio::select! {
            biased;
            () = &mut self.shutdown => Some(Ended::ShutDown),
            () = &mut self.stop => Some(Ended::Stopped),
            () = resumed => None,
        }
    }
}
