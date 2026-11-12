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
use crate::message::{Failure, Quoted, Unavailability};
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
    /// The desktop's settings, asked of the reader as this operation: its
    /// answer carries the number, so an answer to an earlier one, asked
    /// for a window since gone, is not taken for it.
    Configure(u64),
}

impl Pending {
    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }

    /// What it captures, if it is a capture.
    pub(super) fn capture(&self) -> Option<Capture> {
        match self.kind {
            Kind::Capture(capture) => Some(capture),
            Kind::Configure(_) => None,
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

    /// Why no capture can run now, if none can: the desktop delivers the
    /// hotkeys, or the session may not be read (locked: an unlock password
    /// is typed there). Whether a keyboard can be read is the capture
    /// reader's to find out, since it reads more of them: it reports that
    /// at once, which ends the capture.
    fn capture_refusal(&self) -> Option<Unavailability> {
        if self.platform.input_reader() == InputReader::Desktop {
            return Some(Unavailability::DesktopDelivers);
        }
        self.blocked.clone().map(Unavailability::Blocked)
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
            return Some(Outcome::Unavailable(Unavailability::ReadsKeyboards));
        }
        if activation_token
            .as_deref()
            .is_some_and(|token| !plain_token(token))
        {
            return Some(Outcome::Failed(Failure::BadToken));
        }
        let operation = self.settings.next_operation();
        let command = ReaderCommand::ConfigureShortcuts {
            operation,
            activation_token,
        };
        if !self.hotkeys.command(command) {
            return Some(Outcome::Unavailable(Unavailability::NoShortcuts));
        }
        let deadline = Instant::now().checked_add(CONFIGURE_TIME)?;
        self.settings.pending = Some(Pending {
            id,
            deadline,
            kind: Kind::Configure(operation),
        });
        None
    }

    /// The reader captured something. A result that comes at or after the
    /// deadline (the controller was late to look) finds the capture over.
    pub(super) fn captured(&mut self, captured: Captured) {
        self.pending_expired();
        let outcome = match captured {
            Captured::Hotkey(binding) => Outcome::CapturedHotkey(binding),
            Captured::Key(key) => Outcome::CapturedKey(key.code()),
            Captured::Cancelled => Outcome::CaptureEnded(CaptureEnd::Cancelled),
            Captured::Unnamed => Outcome::CaptureEnded(CaptureEnd::Unnamed),
        };
        self.end_capture(outcome);
    }

    /// The reader answered `operation`, a request to open the desktop's
    /// settings; an answer to any other than the one pending, or after its
    /// deadline, is dropped.
    pub(super) fn configured(&mut self, operation: u64, result: Result<(), String>) {
        self.pending_expired();
        let result = result.map_err(|why| Failure::Desktop(Quoted::new(why)));
        self.answer_configure(operation, result);
    }

    /// Answers the request to open the desktop's settings that `operation`
    /// is, if it is the one pending.
    fn answer_configure(&mut self, operation: u64, result: Result<(), Failure>) {
        let ours = |p: &mut Pending| matches!(p.kind, Kind::Configure(asked) if asked == operation);
        let Some(pending) = self.settings.pending.take_if(ours) else {
            return;
        };
        let outcome = match result {
            Ok(()) => Outcome::Configured,
            Err(why) => Outcome::Failed(why),
        };
        self.answer(pending.id, outcome);
    }

    /// Hotkeys stopped working: a capture can get nothing, and the
    /// desktop's settings, asked of a session now gone, will not open.
    pub(super) fn pending_status(&mut self, status: &HotkeyStatus) {
        let HotkeyStatus::Unavailable(why) = status else {
            return;
        };
        let operation = match self.settings.pending.as_ref().map(|p| p.kind) {
            Some(Kind::Configure(operation)) => operation,
            _ => {
                let why = Unavailability::Hotkeys(why.clone());
                return self.end_capture(Outcome::Unavailable(why));
            }
        };
        self.answer_configure(operation, Err(Failure::Hotkeys(why.clone())));
    }

    /// The pending request's deadline passed: it ends unanswered. Other
    /// deadlines (a countdown's seconds) wake the controller too.
    pub(super) fn pending_expired(&mut self) {
        let Some(pending) = &self.settings.pending else {
            return;
        };
        if Instant::now() < pending.deadline {
            return;
        }
        match pending.kind {
            Kind::Capture(_) => self.end_capture(Outcome::CaptureEnded(CaptureEnd::TimedOut)),
            Kind::Configure(operation) => {
                self.answer_configure(operation, Err(Failure::NotOpened));
            }
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
