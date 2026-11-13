//! The pointer's position from KWin (R03, decision 0001 and the R03 brief),
//! and word of a KWin that restarted (W01 brief, W5).
//!
//! A KWin script, embedded here, calls the daemon over D-Bus while a
//! recording runs: it reports the layout at load and on every change of the
//! screens, and the pointer on every move. Nothing outside this crate talks
//! to KWin. The positions reach the recorder through the watcher
//! [`Kwin::watch`] starts, which the controller polls, each with the layout
//! the script reported before it. The screen layout itself comes from the
//! Wayland display (W01b).
//!
//! Calls are accepted only from the current owner of `org.kde.KWin`, with
//! the token of a load this daemon made. That keeps stale scripts and stray
//! callers out; it is no boundary against other processes of the same user,
//! which can load KWin scripts of their own.

mod dir;
mod display;
#[cfg(test)]
mod plasma_tests;
mod receiver;
mod report;
mod script;
mod scripting;
mod session;
mod token;
mod watcher;

use std::cell::RefCell;
use std::future::pending;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use auto_clicker_core::controller::{LayoutSignal, LayoutWatch};
use auto_clicker_core::message::NoPointer;
use auto_clicker_core::recording::RecordingFeed;
use tokio::sync::{Notify, mpsc};
use zbus::Connection;

use dir::ScriptDir;
use receiver::{PATH, QUEUE, Receiver, SharedTrust};
use scripting::{Bus, CALL_TIMEOUT};
use session::Session;
use watcher::{Inbox, Watcher};

/// KWin as the daemon uses it. Clones share one KWin session; one without
/// KWin ([`Kwin::none`]) reports nothing.
#[derive(Clone, Default)]
pub struct Kwin(Option<Rc<Shared>>);

struct Shared {
    session: Rc<Session<Bus>>,
    /// What the watcher reads, until it starts.
    inbox: RefCell<Option<Inbox>>,
}

impl Kwin {
    /// Serves the receiver on `bus`, the connection that holds the daemon's
    /// name, stamping each call with `clock`, the event clock. Calls KWin
    /// for nothing yet: the watcher does, once it runs. Without the
    /// receiver, the daemon runs without positions. KWin counts only when
    /// it is `compositor`, the process listening on the daemon's Wayland
    /// display, or its child; always when that is `None`.
    pub async fn serve(
        bus: &Connection,
        clock: fn() -> Duration,
        compositor: Option<NonZeroU32>,
    ) -> Kwin {
        let trust = SharedTrust::default();
        let (queue, reports) = mpsc::channel(QUEUE);
        let layout_dropped = Arc::new(Notify::new());
        let receiver = Receiver {
            trust: Arc::clone(&trust),
            clock,
            queue,
            layout_dropped: Arc::clone(&layout_dropped),
        };
        if let Err(err) = bus.object_server().at(PATH, receiver).await {
            eprintln!(
                "auto-clicker: cannot receive from KWin, so recordings get no positions: {err}"
            );
            return Kwin::none();
        }
        let dir = ScriptDir::from_env();
        if let Err(why) = &dir {
            eprintln!("auto-clicker: no KWin scripts, so recordings get no positions: {why}");
        }
        let session = Session {
            scripting: Bus(bus.clone()),
            dir,
            trust,
            owner: RefCell::default(),
            compositor,
        };
        let inbox = Inbox {
            reports,
            layout_dropped,
        };
        Kwin(Some(Rc::new(Shared {
            session: Rc::new(session),
            inbox: RefCell::new(Some(inbox)),
        })))
    }

    /// No KWin: nothing is loaded, and recordings get no positions.
    pub fn none() -> Kwin {
        Kwin(None)
    }

    /// Whether KWin reports the pointer to recordings now: a KWin of the
    /// display is counted, and its script can be used. The watcher's
    /// `changed` says when the KWin counted changes.
    pub fn pointer(&self) -> Result<(), NoPointer> {
        match &self.0 {
            Some(shared) => shared.session.pointer(),
            None => Err(NoPointer::Unusable),
        }
    }

    /// The watcher, to be polled on the daemon's thread: it loads the
    /// position script while `feed` records, feeds the recording its
    /// positions, fires `restarted` each time a KWin of the display comes
    /// to own `org.kde.KWin`, the first one seen included, and `changed`
    /// each time the KWin counted changes ([`Kwin::pointer`]). Only the
    /// first call gets it; any later one, like one without KWin, watches
    /// nothing.
    pub fn watch(
        &self,
        feed: RecordingFeed,
        restarted: LayoutSignal,
        changed: LayoutSignal,
    ) -> LayoutWatch {
        let started = self.0.as_ref().and_then(|shared| {
            let inbox = shared.inbox.borrow_mut().take()?;
            let session = Rc::clone(&shared.session);
            let watcher = Watcher::new(session, feed, restarted, changed);
            Some(watcher.run(inbox))
        });
        match started {
            Some(watcher) => Box::pin(watcher),
            None => Box::pin(pending()),
        }
    }

    /// At exit, however the daemon ends once it holds its name: unloads
    /// every script name and removes their files, within two calls' deadlines (asking
    /// who runs KWin, then unloading) and a little more. What fails is
    /// logged; the next start cleans it up.
    pub async fn exit(&self) {
        let Some(shared) = &self.0 else {
            return;
        };
        let deadline = CALL_TIMEOUT
            .saturating_mul(2)
            .saturating_add(Duration::from_millis(500));
        if tokio::time::timeout(deadline, shared.session.exit())
            .await
            .is_err()
        {
            eprintln!(
                "auto-clicker: KWin did not answer at exit; the next start unloads its scripts"
            );
        }
    }
}
