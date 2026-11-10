//! The hotkey reader as the controller holds it: the reader's future and the
//! channel it sends on, replaced together.

use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::task::Poll;

use tokio::sync::mpsc;

use crate::hotkeys::HotkeyEvent;

/// A running hotkey reader. The controller polls it inside its own loop and
/// stops it by dropping it.
pub type HotkeyReader = Pin<Box<dyn Future<Output = ()>>>;

/// Where a hotkey reader sends what it sees. Sending never blocks.
#[derive(Clone, Debug)]
pub struct HotkeySender(mpsc::UnboundedSender<HotkeyEvent>);

impl HotkeySender {
    /// Hands `event` to the controller. An event for a reader the controller
    /// has already dropped is discarded.
    pub fn send(&self, event: HotkeyEvent) {
        // The receiver is gone only when the controller replaced this reader
        // or ended; the event no longer matters then.
        let _ = self.0.send(event);
    }
}

/// One reader and the events it sent that the controller has not read yet.
/// Dropping it stops the reader and discards those events.
pub(super) struct HotkeySource {
    /// `None` once the reader returned (watching for keyboards failed).
    reader: Option<HotkeyReader>,
    events: mpsc::UnboundedReceiver<HotkeyEvent>,
}

impl HotkeySource {
    /// A channel for a reader `start` creates.
    pub(super) fn start(start: impl FnOnce(HotkeySender) -> HotkeyReader) -> HotkeySource {
        let (sender, events) = mpsc::unbounded_channel();
        HotkeySource {
            reader: Some(start(HotkeySender(sender))),
            events,
        }
    }

    /// No reader at all: never yields an event.
    pub(super) fn none() -> HotkeySource {
        let (_, events) = mpsc::unbounded_channel();
        HotkeySource {
            reader: None,
            events,
        }
    }

    /// The next event. Drives the reader while it runs; once it has returned
    /// and its last events are read, never resolves.
    ///
    /// Events already sent come first: a reader kept busy by a flood of key
    /// events can spend the task's budget, after which the channel reports
    /// nothing, and a stop hotkey it sent would wait behind the flood.
    pub(super) fn next(&mut self) -> impl Future<Output = HotkeyEvent> + '_ {
        poll_fn(|cx| {
            if let Poll::Ready(Some(event)) = self.events.poll_recv(cx) {
                return Poll::Ready(event);
            }
            if let Some(reader) = &mut self.reader
                && reader.as_mut().poll(cx).is_ready()
            {
                // A finished future must not be polled again. Dropping it
                // drops its sender, so the channel ends after its events.
                self.reader = None;
            }
            match self.events.poll_recv(cx) {
                Poll::Ready(Some(event)) => Poll::Ready(event),
                Poll::Ready(None) | Poll::Pending => Poll::Pending,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::Action;

    #[tokio::test(start_paused = true)]
    async fn a_reader_kept_busy_cannot_hide_what_it_sent() {
        let fired = HotkeyEvent::Fired(Action::ToggleClicking);
        let sent = fired.clone();
        let mut source = HotkeySource::start(|send| {
            Box::pin(async move {
                send.send(sent);
                // Always more to read: spends the budget on every poll.
                loop {
                    tokio::task::consume_budget().await;
                }
            })
        });
        // Bounded by polls, not time: a busy task never lets the paused
        // clock move.
        let polls = async {
            for _ in 0..100 {
                tokio::task::yield_now().await;
            }
        };
        tokio::select! {
            biased;
            got = source.next() => assert_eq!(got, fired),
            () = polls => panic!("the sent event never came out"),
        }
    }
}
