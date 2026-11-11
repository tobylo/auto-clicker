//! The hotkey reader as the controller holds it: the reader's future and the
//! channel it sends on, replaced together.

use std::convert::Infallible;
use std::future::{Future, pending, poll_fn};
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

/// What the controller asks of a running reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReaderCommand {
    /// Open the desktop's settings for the program's shortcuts, raised with
    /// this activation token if there is one. A reader of the desktop's
    /// shortcuts answers with [`HotkeyEvent::Configured`], carrying
    /// `operation`; a reader of the keyboards has nothing to open and
    /// ignores it.
    ConfigureShortcuts {
        operation: u64,
        activation_token: Option<String>,
    },
}

/// Where a reader gets the controller's commands.
pub struct ReaderCommands(mpsc::UnboundedReceiver<ReaderCommand>);

impl ReaderCommands {
    /// A channel of commands: where they go in, and where a reader gets
    /// them.
    pub fn channel() -> (mpsc::UnboundedSender<ReaderCommand>, ReaderCommands) {
        let (sender, received) = mpsc::unbounded_channel();
        (sender, ReaderCommands(received))
    }

    /// The next command; never resolves once the controller dropped the
    /// reader's other end, which it does only when it drops the reader.
    pub async fn next(&mut self) -> ReaderCommand {
        match self.0.recv().await {
            Some(command) => command,
            None => pending().await,
        }
    }
}

/// One reader and the events it sent that the controller has not read yet.
/// Dropping it stops the reader and discards those events.
pub(super) struct HotkeySource {
    /// `None` once the reader returned (watching for keyboards failed).
    pub(super) reader: Option<HotkeyReader>,
    pub(super) events: mpsc::UnboundedReceiver<HotkeyEvent>,
    /// Where commands to the reader go.
    pub(super) commands: mpsc::UnboundedSender<ReaderCommand>,
}

impl HotkeySource {
    /// Channels for a reader `start` creates.
    pub(super) fn start(
        start: impl FnOnce(HotkeySender, ReaderCommands) -> HotkeyReader,
    ) -> HotkeySource {
        let (sender, events) = mpsc::unbounded_channel();
        let (commands, received) = ReaderCommands::channel();
        HotkeySource {
            reader: Some(start(HotkeySender(sender), received)),
            events,
            commands,
        }
    }

    /// No reader at all: never yields an event, and takes no command.
    pub(super) fn none() -> HotkeySource {
        let (_, events) = mpsc::unbounded_channel();
        let (commands, _) = mpsc::unbounded_channel();
        HotkeySource {
            reader: None,
            events,
            commands,
        }
    }

    /// Hands `command` to the reader; `false` when no reader takes it.
    pub(super) fn command(&self, command: ReaderCommand) -> bool {
        self.reader.is_some() && self.commands.send(command).is_ok()
    }
}

/// The next event a reader sent. Once the reader has returned and its last
/// events are read, never resolves.
pub(super) async fn received(events: &mut mpsc::UnboundedReceiver<HotkeyEvent>) -> HotkeyEvent {
    match events.recv().await {
        Some(event) => event,
        None => pending().await,
    }
}

/// Drives the reader, which reads keyboards and sends what it finds; never
/// resolves. A reader that returned is dropped: a finished future must not
/// be polled again, and dropping it drops its sender, so its channel ends
/// after its last events. The layout watcher is the same kind of future and
/// is driven the same way.
pub(super) fn drive(reader: &mut Option<HotkeyReader>) -> impl Future<Output = Infallible> + '_ {
    poll_fn(move |cx| {
        if let Some(running) = reader
            && running.as_mut().poll(cx).is_ready()
        {
            *reader = None;
        }
        Poll::Pending
    })
}
