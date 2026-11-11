//! What the settings window waits on: a capture of what the user presses,
//! or the desktop's settings for the program's shortcuts. At most one is
//! pending, and each ends by its deadline at the latest.
//!
//! A capture runs only while the controller is settled. It starts the
//! reader again to read every keyboard and fire no hotkey, and ends before
//! anything else happens: a message, a window coming or going, the
//! keyboards becoming unreadable, its deadline or the key it waited for.
//! Each end starts the reader again with the hotkeys, while the devices it
//! must leave out are still the controller's.

use tokio::time::Instant;

use super::hotkeys::{HotkeySource, ReaderCommand};
use super::{Controller, InputReader, Message, Platform};
use crate::hotkeys::{Capture, Captured, HotkeyStatus};
use crate::window::{Answer, CAPTURE_TIME, CaptureEnd, MAX_TOKEN, Outcome};

/// How long the desktop may take to open its shortcut settings. The reader
/// bounds its own call by less; this ends the wait if the reader is gone.
const CONFIGURE_TIME: std::time::Duration = std::time::Duration::from_secs(60);

/// A request the window waits on.
pub(super) struct Pending {
    /// The request it answers.
    id: u64,
    /// When it ends without an answer of its own.
    deadline: Instant,
    kind: Kind,
}

#[derive(Clone, Copy)]
enum Kind {
    Capture(Capture),
    Configure,
}

impl Pending {
    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }

    /// What it captures, if it is a capture.
    pub(super) fn capture(&self) -> Option<Capture> {
        match self.kind {
            Kind::Capture(capture) => Some(capture),
            Kind::Configure => None,
        }
    }
}

/// Whether `message` ends a capture: everything but what changes nothing
/// the controller does.
pub(super) fn ends_capture(message: &Message) -> bool {
    !matches!(message, Message::MenuOpened | Message::LayoutChanged)
}

impl<P: Platform> Controller<P> {
    /// Starts a capture for request `id`; the answer comes when it ends.
    pub(super) fn start_capture(&mut self, id: u64, capture: Capture) -> Option<Outcome> {
        if !self.settled() {
            return Some(Outcome::Busy);
        }
        if let Some(why) = self.capture_refusal() {
            return Some(Outcome::Unavailable(why));
        }
        let deadline = Instant::now().checked_add(CAPTURE_TIME)?;
        self.settings.pending = Some(Pending {
            id,
            deadline,
            kind: Kind::Capture(capture),
        });
        self.restart_reader();
        None
    }

    /// Why no capture can run now, if none can.
    fn capture_refusal(&self) -> Option<String> {
        if self.platform.input_reader() == InputReader::Desktop {
            return Some(
                "the desktop delivers the hotkeys: change them in System Settings, under \
                 Shortcuts"
                    .to_owned(),
            );
        }
        match &self.hotkey_status {
            Some(HotkeyStatus::Unavailable(why)) => Some(why.to_string()),
            _ if self.hotkeys.reader.is_none() => Some("the keyboards are not read".to_owned()),
            _ => None,
        }
    }

    /// Asks the reader to open the desktop's shortcut settings for request
    /// `id`; the answer comes from the reader, or at the deadline.
    pub(super) fn configure_shortcuts(
        &mut self,
        id: u64,
        activation_token: Option<String>,
    ) -> Option<Outcome> {
        if self.settings.pending.is_some() {
            return Some(Outcome::Busy);
        }
        if self.platform.input_reader() != InputReader::Desktop {
            return Some(Outcome::Unavailable(
                "the program reads the keyboards: set the hotkeys in this window".to_owned(),
            ));
        }
        if activation_token
            .as_deref()
            .is_some_and(|token| !plain_token(token))
        {
            return Some(Outcome::Failed(
                "the activation token is not one".to_owned(),
            ));
        }
        let command = ReaderCommand::ConfigureShortcuts { activation_token };
        if !self.hotkeys.command(command) {
            return Some(Outcome::Unavailable(
                "the desktop's shortcuts are not available".to_owned(),
            ));
        }
        let deadline = Instant::now().checked_add(CONFIGURE_TIME)?;
        self.settings.pending = Some(Pending {
            id,
            deadline,
            kind: Kind::Configure,
        });
        None
    }

    /// The reader captured something.
    pub(super) fn captured(&mut self, captured: Captured) {
        let outcome = match captured {
            Captured::Hotkey(binding) => Outcome::CapturedHotkey(binding),
            Captured::Key(key) => Outcome::CapturedKey(key.code()),
            Captured::Cancelled => Outcome::CaptureEnded(CaptureEnd::Cancelled),
            Captured::Unnamed => Outcome::CaptureEnded(CaptureEnd::Unnamed),
        };
        self.end_capture(outcome);
    }

    /// The reader answered a request to open the desktop's settings.
    pub(super) fn configured(&mut self, result: Result<(), String>) {
        let Some(pending) = self.settings.pending.take_if(|p| p.capture().is_none()) else {
            return;
        };
        let outcome = match result {
            Ok(()) => Outcome::Configured,
            Err(why) => Outcome::Failed(why),
        };
        self.answer(pending.id, outcome);
    }

    /// Hotkeys stopped working during a capture: it can get nothing.
    pub(super) fn capture_status(&mut self, status: &HotkeyStatus) {
        if let HotkeyStatus::Unavailable(why) = status {
            self.end_capture(Outcome::Unavailable(why.to_string()));
        }
    }

    /// The pending request's deadline passed: it ends unanswered.
    pub(super) fn pending_expired(&mut self) {
        let expired = self
            .settings
            .pending
            .as_ref()
            .map(|p| (p.deadline, p.capture()));
        match expired {
            Some((deadline, Some(_))) if Instant::now() >= deadline => {
                self.end_capture(Outcome::CaptureEnded(CaptureEnd::TimedOut));
            }
            Some((deadline, None)) if Instant::now() >= deadline => {
                self.configured(Err("the desktop did not open its settings".to_owned()));
            }
            _ => {}
        }
    }

    /// Ends a capture, if one runs, with `outcome`, and starts the reader
    /// again with the hotkeys.
    pub(super) fn end_capture(&mut self, outcome: Outcome) {
        let Some(pending) = self.settings.pending.take_if(|p| p.capture().is_some()) else {
            return;
        };
        self.answer(pending.id, outcome);
        self.restart_reader();
    }

    /// Ends whatever the window waits on, for a window that went: the
    /// desktop's settings stay open, but nobody waits for them.
    pub(super) fn end_pending(&mut self) {
        self.end_capture(Outcome::CaptureEnded(CaptureEnd::Interrupted));
        self.settings.pending = None;
    }

    fn answer(&mut self, id: u64, outcome: Outcome) {
        self.settings.answer = Some(Answer {
            id,
            outcome: outcome.bounded(),
        });
    }

    /// Drops the reader in use, then starts one for what the settings ask
    /// now: a capture, or the hotkeys. Two never read at once.
    pub(super) fn restart_reader(&mut self) {
        self.hotkeys = HotkeySource::none();
        self.start_reader();
    }
}

/// Whether an activation token can be passed on: short, and printable.
fn plain_token(token: &str) -> bool {
    token.len() <= MAX_TOKEN && token.chars().all(|c| c.is_ascii_graphic())
}
