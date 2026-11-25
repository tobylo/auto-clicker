//! What the message thread hears, turned into the controller's messages
//! (M5 brief decision 15): a lock, a switch to another user, sleep and the
//! display turning off stop a run, as logind's facts do on Linux; logoff
//! and a request to close quit. The thread only sends; the controller
//! decides.
//!
//! Two notices make the thread wait, each with its own deadline, since
//! Windows goes on once the window procedure returns: before sleep it waits
//! until the controller reports that nothing is held, and at logoff until
//! the controller has ended. Neither the controller's stop nor its release
//! ever waits on the message thread, so neither wait can deadlock.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use auto_clicker_backend_windows::{SessionChange, SessionState, WindowEvent};
use auto_clicker_core::controller::{Blocked, Message, SessionFacts};
use auto_clicker_core::log;
use tokio::sync::mpsc;

/// How long the thread waits before sleep for the controller to let go of
/// everything. Windows suspends about two seconds after it tells programs.
pub const RELEASE_BEFORE_SLEEP: Duration = Duration::from_secs(1);
/// How long the thread waits at logoff for the controller to end. Windows
/// may end the process once the window procedure returns, and shows its
/// "this app is preventing shutdown" screen after about five seconds.
pub const RELEASE_BEFORE_LOGOFF: Duration = Duration::from_secs(3);

/// A count that only goes up, which one thread waits on and another
/// raises: the controller's reports to the message thread.
#[derive(Debug, Default)]
pub struct Mark {
    reached: Mutex<u64>,
    raised: Condvar,
}

impl Mark {
    /// Raises the mark to `to`; a lower value changes nothing.
    pub fn raise(&self, to: u64) {
        let mut reached = self.reached.lock().unwrap_or_else(PoisonError::into_inner);
        if to > *reached {
            *reached = to;
            self.raised.notify_all();
        }
    }

    /// Waits until the mark is at least `at_least`, at most `within`;
    /// whether it got there.
    pub fn wait_for(&self, at_least: u64, within: Duration) -> bool {
        let reached = self.reached.lock().unwrap_or_else(PoisonError::into_inner);
        let (reached, _) = self
            .raised
            .wait_timeout_while(reached, within, |reached| *reached < at_least)
            .unwrap_or_else(PoisonError::into_inner);
        *reached >= at_least
    }
}

/// The message thread's handler: it folds what Windows says into session
/// facts and sends the controller what changed.
pub struct Notices {
    session: Session,
    send: mpsc::UnboundedSender<Message>,
    /// The last sleep notice the controller let go for.
    released: Arc<Mark>,
    /// Raised to 1 once the controller has ended, with nothing held.
    ended: Arc<Mark>,
}

impl Notices {
    pub fn new(
        send: mpsc::UnboundedSender<Message>,
        released: Arc<Mark>,
        ended: Arc<Mark>,
    ) -> Notices {
        Notices {
            session: Session::default(),
            send,
            released,
            ended,
        }
    }

    /// Handles one event from the message thread.
    pub fn handle(&mut self, event: WindowEvent) {
        match &event {
            WindowEvent::EndSession => {
                self.send(Message::Quit);
                if !self.ended.wait_for(1, RELEASE_BEFORE_LOGOFF) {
                    log::line(format_args!(
                        "auto-clicker: the session ends before the daemon did"
                    ));
                }
                return;
            }
            WindowEvent::Close => return self.send(Message::Quit),
            WindowEvent::Ended { failure } => {
                if let Some(failure) = failure {
                    log::line(format_args!("auto-clicker: {failure}; quitting"));
                }
                return self.send(Message::Quit);
            }
            WindowEvent::Other(message) => {
                return log::line(format_args!(
                    "auto-clicker: woken by window message {message:#06x}"
                ));
            }
            WindowEvent::DisplayUnwatched(err) => {
                return log::line(format_args!(
                    "auto-clicker: the display's state cannot be followed: {err}"
                ));
            }
            _ => {}
        }
        match self.session.apply(&event) {
            Told::Nothing => {}
            Told::Session(blocked) => {
                if let Some(Blocked::Unknown(why)) = &blocked {
                    log::line(format_args!("auto-clicker: {why}"));
                }
                self.send(Message::Session(blocked));
            }
            Told::Sleep(notice) => {
                self.send(Message::Sleep(notice));
                if !self.released.wait_for(notice, RELEASE_BEFORE_SLEEP) {
                    log::line(format_args!(
                        "auto-clicker: sleep came before the daemon let go of everything"
                    ));
                }
            }
        }
    }

    fn send(&self, message: Message) {
        // The controller is gone only once it ended; nothing is left to tell.
        let _ = self.send.send(message);
    }
}

/// What the controller is to be told after an event.
#[derive(Debug, PartialEq)]
enum Told {
    Nothing,
    /// Why clicking may not run now, or `None` when it may.
    Session(Option<Blocked>),
    /// The machine is going to sleep (or its display off): the n-th time.
    Sleep(u64),
}

/// The session's facts as the message thread last heard them.
#[derive(Debug, Default)]
struct Session {
    /// `None` until read, or when Windows did not say.
    locked: Option<bool>,
    /// `None` until read.
    active: Option<bool>,
    /// Why the state is not known, when it is not.
    unknown: Option<String>,
    suspended: bool,
    display_off: bool,
    /// What the controller was last told; `None` before anything was.
    told: Option<Option<Blocked>>,
    /// How many times sleep was announced.
    sleeps: u64,
}

impl Session {
    /// Folds `event` in; what the controller must be told about it.
    fn apply(&mut self, event: &WindowEvent) -> Told {
        let was_asleep = self.asleep();
        match event {
            WindowEvent::SessionRead(Ok(state)) => self.read(*state),
            WindowEvent::SessionRead(Err(err)) => {
                self.forget(format!("the session state could not be read: {err}"));
            }
            WindowEvent::SessionUnwatched(err) => {
                self.forget(format!("Windows' session notices are not available: {err}"));
            }
            WindowEvent::Session(change) => self.change(*change),
            WindowEvent::Suspend => self.suspended = true,
            WindowEvent::Resume => self.suspended = false,
            WindowEvent::Display(on) => self.display_off = !on,
            _ => return Told::Nothing,
        }
        if self.asleep() && !was_asleep {
            self.sleeps = self.sleeps.saturating_add(1);
            self.told = Some(Some(Blocked::Sleeping));
            return Told::Sleep(self.sleeps);
        }
        let blocked = self.blocked();
        if self.told.as_ref() == Some(&blocked) {
            return Told::Nothing;
        }
        self.told = Some(blocked.clone());
        Told::Session(blocked)
    }

    fn read(&mut self, state: SessionState) {
        self.locked = state.locked;
        self.active = Some(state.active);
        self.unknown = state
            .locked
            .is_none()
            .then(|| "Windows does not say whether the session is locked".to_owned());
    }

    /// A definite notice: it holds whatever was not known before.
    fn change(&mut self, change: SessionChange) {
        match change {
            SessionChange::Lock => self.locked = Some(true),
            // Only the session at the screen can be unlocked.
            SessionChange::Unlock => {
                self.locked = Some(false);
                self.active = Some(true);
            }
            SessionChange::Connect => self.active = Some(true),
            SessionChange::Disconnect => self.active = Some(false),
        }
        if self.locked.is_some() && self.active.is_some() {
            self.unknown = None;
        }
    }

    /// Nothing is known any more, for `why`.
    fn forget(&mut self, why: String) {
        self.locked = None;
        self.active = None;
        self.unknown = Some(why);
    }

    fn asleep(&self) -> bool {
        self.suspended || self.display_off
    }

    /// Why clicking may not run, or `None` when it may: sleep first, then
    /// a state not known, then core's order of the lock and another
    /// session in front.
    fn blocked(&self) -> Option<Blocked> {
        if self.asleep() {
            return Some(Blocked::Sleeping);
        }
        match (self.locked, self.active) {
            (Some(locked), Some(active)) => SessionFacts {
                locked,
                active,
                sleeping: false,
            }
            .blocked(),
            _ => Some(Blocked::Unknown(
                self.unknown
                    .clone()
                    .unwrap_or_else(|| "the session state is not read yet".to_owned())
                    .into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests;
