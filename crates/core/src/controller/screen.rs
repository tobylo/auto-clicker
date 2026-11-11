//! The screen layout's watcher as the controller holds it: the watcher's
//! future and the signal it fires when the layout changed.
//!
//! The platform starts the watcher once ([`Platform::watch_layout`]), and the
//! controller polls it inside its own loop, as it polls the hotkey reader.
//! The watcher only reports: it hands positions to a recording that runs and
//! fires its signal when [`Platform::layout`] changed. A fired signal is
//! handled as [`Message::LayoutChanged`] is, so the watcher needs no way into
//! the controller's inbox, and starts and stops nothing.
//!
//! [`Platform::watch_layout`]: super::Platform::watch_layout
//! [`Platform::layout`]: super::Platform::layout
//! [`Message::LayoutChanged`]: super::Message::LayoutChanged

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use tokio::sync::Notify;

/// A running layout watcher. The controller polls it and drops it when it
/// returns or the controller ends.
pub type LayoutWatch = Pin<Box<dyn Future<Output = ()>>>;

/// How a layout watcher tells the controller the layout changed. Firing it
/// never blocks; fires the controller has not handled yet count as one.
#[derive(Clone, Debug, Default)]
pub struct LayoutSignal(Rc<Notify>);

impl LayoutSignal {
    /// [`Platform::layout`](super::Platform::layout) changed, or became
    /// unknown.
    pub fn changed(&self) {
        self.0.notify_one();
    }

    /// Waits until the signal fired since this was last waited for.
    pub async fn fired(&self) {
        self.0.notified().await;
    }
}

/// The watcher and its signal, held together.
#[derive(Default)]
pub(super) struct LayoutSource {
    /// `None` until it starts, and once it returned.
    pub(super) watch: Option<LayoutWatch>,
    pub(super) signal: LayoutSignal,
}

impl LayoutSource {
    /// Starts the watcher `start` makes with this source's signal.
    pub(super) fn start(&mut self, start: impl FnOnce(LayoutSignal) -> LayoutWatch) {
        self.watch = Some(start(self.signal.clone()));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn fires_before_a_wait_are_kept_and_count_as_one() {
        let signal = LayoutSignal::default();
        let quiet = timeout(Duration::from_secs(1), signal.fired()).await;
        assert!(quiet.is_err(), "fired before anything changed");
        signal.changed();
        signal.clone().changed();
        timeout(Duration::from_secs(1), signal.fired())
            .await
            .unwrap();
        let again = timeout(Duration::from_secs(1), signal.fired()).await;
        assert!(again.is_err(), "two fires woke it twice");
    }
}
