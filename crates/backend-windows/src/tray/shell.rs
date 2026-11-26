//! The thread that makes the tray's shell calls (M5 brief decision 4 as
//! amended by WN07). `Shell_NotifyIconW` waits for Explorer without a limit
//! on Windows 11 (58 s and counting in the VM with Explorer suspended), so
//! no other thread makes it: the message thread, which must hear a lock or
//! a sleep at once, hands each change to this one and goes on.
//!
//! It keeps the icon in step with the latest picture and name: adds it,
//! changes it, tries again on the message thread's ladder while a taskbar
//! refuses it, and adds it afresh when Explorer restarts. Between changes
//! it sleeps on a condition variable; a try that is due is its only
//! deadline. What the user should know (the icon refused, then shown
//! again) goes back to the message thread's handler.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::message_thread::{RETRY_DELAYS, WindowEvent};

/// What the icon shows: its picture and its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    pub title: String,
    pub icon: super::TrayIcon,
}

/// The calls the thread makes, behind a trait so its tests need no
/// taskbar.
pub(super) trait Calls {
    /// Adds the icon and sets its version.
    fn add(&mut self, shown: &Shown) -> io::Result<()>;
    /// Changes the icon added before.
    fn modify(&mut self, shown: &Shown) -> io::Result<()>;
    /// Sets the version of an icon that is there.
    fn version(&mut self) -> io::Result<()>;
    fn delete(&mut self);
    /// Whether a taskbar is there to show the icon.
    fn taskbar_present(&mut self) -> bool;
    /// Tells the message thread's handler `event`.
    fn tell(&mut self, event: WindowEvent);
}

/// What the message thread asked for since the shell thread last looked.
#[derive(Default)]
struct Inbox {
    /// The latest picture and name, if they changed.
    show: Option<Shown>,
    /// Explorer started again.
    restarted: bool,
    /// The window goes: remove the icon and end.
    remove: bool,
}

impl Inbox {
    /// Whether it asks for anything.
    fn asks(&self) -> bool {
        self.show.is_some() || self.restarted || self.remove
    }
}

/// The message thread's side: where it hands changes over.
pub(super) struct Shell {
    shared: Arc<(Mutex<Inbox>, Condvar)>,
    /// Told on the thread's way out.
    done: std::sync::mpsc::Receiver<()>,
    thread: JoinHandle<()>,
}

impl Shell {
    /// Starts the thread, which makes `calls`.
    pub(super) fn spawn(calls: impl Calls + Send + 'static) -> io::Result<Shell> {
        let shared = Arc::new((Mutex::new(Inbox::default()), Condvar::new()));
        let theirs = Arc::clone(&shared);
        let (finished, done) = std::sync::mpsc::channel();
        let thread = thread::Builder::new()
            .name("tray shell".to_owned())
            .spawn(move || {
                Icon::new(calls).run(&theirs);
                let _ = finished.send(());
            })?;
        Ok(Shell {
            shared,
            done,
            thread,
        })
    }

    /// Shows `shown` once the thread gets to it; anything given meanwhile
    /// it replaces.
    pub(super) fn show(&self, shown: Shown) {
        self.ask(|inbox| inbox.show = Some(shown));
    }

    /// Explorer started again: the icon went with the old one.
    pub(super) fn restarted(&self) {
        self.ask(|inbox| inbox.restarted = true);
    }

    /// Removes the icon and ends the thread, waiting at most `within`; a
    /// thread still in a call to a hung Explorer is left to end with the
    /// process, and the icon with it.
    pub(super) fn remove(self, within: Duration) {
        self.ask(|inbox| inbox.remove = true);
        if self.done.recv_timeout(within).is_ok() {
            let _ = self.thread.join();
        }
    }

    fn ask(&self, change: impl FnOnce(&mut Inbox)) {
        let (inbox, changed) = &*self.shared;
        change(&mut inbox.lock().unwrap_or_else(PoisonError::into_inner));
        changed.notify_one();
    }
}

/// Where the icon stands with the taskbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Not added yet, or Explorer restarted: the next look adds it.
    Absent,
    Shown,
    /// The taskbar refused it while there: tried again at `due`.
    Retrying {
        due: Instant,
    },
    /// No taskbar to show it on: added when Explorer says it started.
    Waiting,
}

/// The shell thread's state.
struct Icon<C: Calls> {
    calls: C,
    state: State,
    /// The latest picture and name.
    shown: Option<Shown>,
    /// How many tries the ladder has scheduled since the icon last showed.
    tries: usize,
}

impl<C: Calls> Icon<C> {
    fn new(calls: C) -> Icon<C> {
        Icon {
            calls,
            state: State::Absent,
            shown: None,
            tries: 0,
        }
    }

    /// Takes each change as it comes until told to remove the icon.
    fn run(mut self, shared: &(Mutex<Inbox>, Condvar)) {
        loop {
            let inbox = self.wait(shared);
            if inbox.remove {
                self.remove();
                return;
            }
            self.take(inbox, Instant::now());
        }
    }

    /// Sleeps until the message thread asks for something, or a try is
    /// due; what it asked for.
    fn wait(&self, (inbox, changed): &(Mutex<Inbox>, Condvar)) -> Inbox {
        let mut asked = inbox.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if asked.asks() {
                return std::mem::take(&mut *asked);
            }
            asked = match self.state {
                State::Retrying { due } => {
                    let left = due.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Inbox::default();
                    }
                    changed
                        .wait_timeout(asked, left)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
                _ => changed.wait(asked).unwrap_or_else(PoisonError::into_inner),
            };
        }
    }

    /// Acts on what was asked, at `now`.
    fn take(&mut self, inbox: Inbox, now: Instant) {
        let changed = inbox.show.is_some();
        if let Some(shown) = inbox.show {
            self.shown = Some(shown);
        }
        // Explorer started: whatever showed went with the old one, so the
        // icon is added afresh, with the latest picture.
        if inbox.restarted {
            self.tries = 0;
            return self.add(now);
        }
        match self.state {
            State::Absent => self.add(now),
            State::Shown if changed => self.modify(now),
            State::Retrying { due } if due <= now => self.retry(now),
            State::Shown | State::Retrying { .. } | State::Waiting => {}
        }
    }

    fn add(&mut self, now: Instant) {
        let Some(shown) = self.shown.clone() else {
            return;
        };
        match self.calls.add(&shown) {
            Ok(()) => self.shown_now(),
            Err(err) => self.failed(err, now),
        }
    }

    fn modify(&mut self, now: Instant) {
        let Some(shown) = self.shown.clone() else {
            return;
        };
        if let Err(err) = self.calls.modify(&shown) {
            self.failed(err, now);
        }
    }

    /// A try the ladder scheduled: the icon may be there after all (a call
    /// that failed may have gone through), so it is changed first and added
    /// only when that fails.
    fn retry(&mut self, now: Instant) {
        let Some(shown) = self.shown.clone() else {
            return;
        };
        let changed = self
            .calls
            .modify(&shown)
            .and_then(|()| self.calls.version());
        match changed {
            Ok(()) => self.shown_now(),
            Err(_) => self.add(now),
        }
    }

    fn shown_now(&mut self) {
        let before = std::mem::replace(&mut self.state, State::Shown);
        self.tries = 0;
        if matches!(before, State::Retrying { .. } | State::Waiting) {
            self.calls.tell(WindowEvent::TrayShown);
        }
    }

    /// The taskbar refused the icon: tried again on the ladder while a
    /// taskbar is there, else once Explorer says it started.
    fn failed(&mut self, err: io::Error, now: Instant) {
        self.state = if self.calls.taskbar_present() {
            let delay = RETRY_DELAYS[self.tries.min(RETRY_DELAYS.len() - 1)];
            self.tries = self.tries.saturating_add(1);
            State::Retrying {
                due: now.checked_add(delay).unwrap_or(now),
            }
        } else {
            State::Waiting
        };
        self.calls.tell(WindowEvent::TrayUnshown(err));
    }

    /// Removes the icon, unless it was never added or went with Explorer.
    fn remove(&mut self) {
        if self.state != State::Absent {
            self.calls.delete();
        }
    }
}

/// Events the shell thread has for the message thread's handler, which it
/// takes when told to look.
#[derive(Clone, Default)]
pub(super) struct Outbox(Arc<Mutex<VecDeque<WindowEvent>>>);

impl Outbox {
    pub(super) fn push(&self, event: WindowEvent) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(event);
    }

    pub(super) fn take(&self) -> VecDeque<WindowEvent> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[cfg(test)]
mod tests;
