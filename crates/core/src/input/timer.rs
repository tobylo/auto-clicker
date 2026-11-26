//! How a run waits for its deadlines: through a timer its backend chooses.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::time::{Instant, Sleep, sleep_until};

use super::InputError;

/// One run's timer: the only way a run waits for a deadline.
///
/// A backend makes one at the start of each run
/// ([`InputBackend::run_timer`](super::InputBackend::run_timer)), and the run
/// drops it when it ends, however it ends. On Linux and in tests it is
/// tokio's own sleep ([`TokioTimer`]); on Windows it is a high-resolution
/// waitable timer on a thread that exists only while the run does.
pub trait RunTimer: Send {
    /// Ready once `deadline` has passed. A call with a different deadline
    /// replaces the one before: only the latest deadline can complete a wait,
    /// and a wake meant for an earlier one only polls again. An error means
    /// the timer stopped working, which ends the run.
    fn poll_until(
        &mut self,
        cx: &mut Context<'_>,
        deadline: Instant,
    ) -> Poll<Result<(), InputError>>;
}

/// The run timer of Linux's backends and of every test backend: tokio's
/// `sleep_until`, so engine tests keep running on tokio's paused clock.
#[derive(Debug, Default)]
pub struct TokioTimer {
    sleep: Option<Pin<Box<Sleep>>>,
}

impl TokioTimer {
    /// A new timer, boxed as [`InputBackend::run_timer`](super::InputBackend::run_timer)
    /// returns it.
    pub fn boxed() -> Box<dyn RunTimer> {
        Box::new(TokioTimer::default())
    }
}

impl RunTimer for TokioTimer {
    fn poll_until(
        &mut self,
        cx: &mut Context<'_>,
        deadline: Instant,
    ) -> Poll<Result<(), InputError>> {
        let sleep = match &mut self.sleep {
            Some(sleep) => {
                if sleep.deadline() != deadline {
                    sleep.as_mut().reset(deadline);
                }
                sleep
            }
            None => self.sleep.insert(Box::pin(sleep_until(deadline))),
        };
        sleep.as_mut().poll(cx).map(Ok)
    }
}

#[cfg(test)]
mod tests {
    use std::future::poll_fn;
    use std::time::Duration;

    use super::*;

    async fn wait(timer: &mut dyn RunTimer, deadline: Instant) {
        poll_fn(|cx| timer.poll_until(cx, deadline)).await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn waits_until_each_deadline() {
        let mut timer = TokioTimer::default();
        let start = Instant::now();
        for step in 1..=3u64 {
            let deadline = start + Duration::from_millis(10 * step);
            wait(&mut timer, deadline).await;
            assert_eq!(Instant::now(), deadline);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_deadline_replaces_the_one_before() {
        let mut timer = TokioTimer::default();
        let start = Instant::now();
        let far = start + Duration::from_secs(10);
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        assert!(timer.poll_until(&mut cx, far).is_pending());
        let near = start + Duration::from_millis(5);
        wait(&mut timer, near).await;
        assert_eq!(Instant::now(), near);
    }

    #[tokio::test(start_paused = true)]
    async fn a_past_deadline_is_ready_at_once() {
        let mut timer = TokioTimer::default();
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let past = Instant::now();
        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(timer.poll_until(&mut cx, past).is_ready());
    }
}
