//! What the message thread hears, turned into the controller's messages
//! (M5 brief decision 15): a lock, a switch to another user, sleep and the
//! display turning off stop a run, as logind's facts do on Linux; logoff
//! and a request to close quit. The thread only sends; the controller
//! decides.
//!
//! Two notices make the thread wait, each with its own deadline, since
//! Windows goes on once the window procedure returns: before sleep it waits
//! until the controller reports that nothing is held, and at logoff until
//! the controller has ended. Both wait on one [`Mark`]: the controller's
//! reports raise it, and its end raises it to [`ENDED`], which satisfies
//! every wait. Neither the controller's stop nor its release ever waits on
//! the message thread, so neither wait can deadlock.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use auto_clicker_backend_windows::{SessionChange, SessionState, WindowEvent};
use auto_clicker_core::controller::{Blocked, Message, SessionFacts};
use auto_clicker_core::log;
use tokio::sync::mpsc;

use super::tray::Commands;

/// How long the thread waits before sleep for the controller to let go of
/// everything. Windows suspends about two seconds after it tells programs.
pub const RELEASE_BEFORE_SLEEP: Duration = Duration::from_secs(1);
/// How long the thread waits at logoff for the controller to end. Windows
/// may end the process once the window procedure returns, and shows its
/// "this app is preventing shutdown" screen after about five seconds.
pub const RELEASE_BEFORE_LOGOFF: Duration = Duration::from_secs(3);
/// The mark once the controller has ended, with nothing held: above every
/// sleep notice.
pub const ENDED: u64 = u64::MAX;

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
/// facts and sends the controller what changed, and sends it what the
/// tray's menu asks for.
pub struct Notices {
    session: Session,
    send: mpsc::UnboundedSender<Message>,
    /// The last sleep notice the controller let go for, or [`ENDED`].
    released: Arc<Mark>,
    /// The menu's commands, shared with the tray that draws it.
    commands: Commands,
    /// Whether the taskbar refuses the icon since it last showed: each try
    /// is reported, and only the first is logged.
    icon_refused: bool,
}

impl Notices {
    pub fn new(send: mpsc::UnboundedSender<Message>, released: Arc<Mark>) -> Notices {
        Notices {
            session: Session::default(),
            send,
            released,
            commands: Commands::default(),
            icon_refused: false,
        }
    }

    /// The menu's commands, for the tray to draw the menu with.
    pub fn commands(&self) -> Commands {
        self.commands.clone()
    }

    /// Handles one event from the message thread. The controller is told,
    /// and any wait is made, before anything is logged: a log write never
    /// holds up a stop.
    pub fn handle(&mut self, event: WindowEvent) {
        if self.tray(&event) {
            return;
        }
        match &event {
            WindowEvent::EndSession => {
                self.send(Message::Quit);
                if !self.released.wait_for(ENDED, RELEASE_BEFORE_LOGOFF) {
                    log::line(format_args!(
                        "auto-clicker: the session ends before the daemon did"
                    ));
                }
                return;
            }
            WindowEvent::Close => return self.send(Message::Quit),
            WindowEvent::Ended { failure } => {
                self.send(Message::Quit);
                if let Some(failure) = failure {
                    log::line(format_args!("auto-clicker: {failure}; quitting"));
                }
                return;
            }
            WindowEvent::Other(message) => {
                return log::line(format_args!(
                    "auto-clicker: woken by window message {message:#06x}"
                ));
            }
            _ => {}
        }
        let told = self.session.apply(&event);
        self.tell(told);
        // What Windows said is logged, so that every wake it causes is
        // named: no keystroke is among these.
        if matches!(
            event,
            WindowEvent::Session(_)
                | WindowEvent::Suspend
                | WindowEvent::Resume
                | WindowEvent::Display(_)
        ) {
            log::line(format_args!("auto-clicker: heard {event:?}"));
        }
    }

    /// Tells the controller what changed, and before sleep waits for it to
    /// let go; then logs what went wrong, if anything did.
    fn tell(&self, told: Told) {
        match told {
            Told::Nothing => {}
            Told::Session(blocked) => {
                let unknown = match &blocked {
                    Some(Blocked::Unknown(why)) => Some(why.to_string()),
                    _ => None,
                };
                self.send(Message::Session(blocked));
                if let Some(why) = unknown {
                    log::line(format_args!("auto-clicker: {why}"));
                }
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

    /// Handles `event` if it is the tray's; whether it was. Hovering and
    /// clicking the icon is the user's hand, not a wake, and is not logged.
    fn tray(&mut self, event: &WindowEvent) -> bool {
        match event {
            // A stop chosen from the menu trims the trip from the click
            // that opened it, as on Linux.
            WindowEvent::TrayMenuOpening => self.send(Message::MenuOpened),
            WindowEvent::TrayChosen(command) => {
                if let Some(message) = self.commands.message(*command) {
                    self.send(message);
                }
            }
            WindowEvent::TrayMenuFailed(err) => {
                log::line(format_args!(
                    "auto-clicker: the tray's menu could not open: {err}"
                ));
            }
            WindowEvent::TrayUnshown(err) => {
                if !std::mem::replace(&mut self.icon_refused, true) {
                    log::line(format_args!(
                        "auto-clicker: the tray icon is not shown: {err}; trying again"
                    ));
                }
            }
            WindowEvent::TrayShown => {
                self.icon_refused = false;
                log::line(format_args!("auto-clicker: the tray icon is shown again"));
            }
            _ => return false,
        }
        true
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
    /// Why the display's state cannot be followed, when it cannot: then
    /// the display could turn off unheard, so starts are refused.
    display_unwatched: Option<String>,
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
            WindowEvent::DisplayUnwatched(err) => {
                self.display_unwatched =
                    Some(format!("the display's state cannot be followed: {err}"));
            }
            _ => return Told::Nothing,
        }
        if self.announces_sleep(event, was_asleep) {
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

    /// Whether `event` announces sleep: it put the machine to sleep, or it
    /// is a suspend. Each suspend is announced, even in the dark, so the
    /// thread waits for the controller's report before Windows suspends,
    /// whatever became of an earlier wait.
    fn announces_sleep(&self, event: &WindowEvent, was_asleep: bool) -> bool {
        let suspending = matches!(event, WindowEvent::Suspend);
        self.asleep() && (suspending || !was_asleep)
    }

    fn asleep(&self) -> bool {
        self.suspended || self.display_off
    }

    /// Why clicking may not run, or `None` when it may: sleep first, then
    /// a session state not known, then a display not followed, then core's
    /// order of the lock and another session in front.
    fn blocked(&self) -> Option<Blocked> {
        if self.asleep() {
            return Some(Blocked::Sleeping);
        }
        let (Some(locked), Some(active)) = (self.locked, self.active) else {
            let why = self
                .unknown
                .as_deref()
                .unwrap_or("the session state is not read yet");
            return Some(Blocked::Unknown(why.to_owned().into()));
        };
        if let Some(why) = &self.display_unwatched {
            return Some(Blocked::Unknown(why.clone().into()));
        }
        SessionFacts {
            locked,
            active,
            sleeping: false,
        }
        .blocked()
    }
}

#[cfg(test)]
mod tests;
