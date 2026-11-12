//! The screen layout and the pointer's position from KWin (R03, decision
//! 0001 and the R03 brief).
//!
//! Two KWin scripts, embedded here, call the daemon over D-Bus: the layout
//! script, loaded for the daemon's lifetime, reports every output's
//! rectangle and scale; the position script, loaded only while a recording
//! runs, reports the layout once and the pointer on every move. Nothing
//! outside this crate talks to KWin. The daemon sees the current layout
//! through [`Kwin::layout`], and the positions reach the recorder through
//! the watcher [`Kwin::watch`] starts, which the controller polls.
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
use auto_clicker_core::recording::{Layout, RecordingFeed};
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
            layout: RefCell::default(),
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

    /// No KWin: the layout is never known, and nothing is loaded.
    pub fn none() -> Kwin {
        Kwin(None)
    }

    /// The current layout; `None` while it is not known.
    pub fn layout(&self) -> Option<Layout> {
        self.0.as_ref()?.session.layout.borrow().clone()
    }

    /// The watcher, to be polled on the daemon's thread: it loads the
    /// scripts, feeds `feed`'s recordings their positions and fires
    /// `changed` when the layout changes. Only the first call gets it; any
    /// later one, like one without KWin, watches nothing.
    pub fn watch(&self, feed: RecordingFeed, changed: LayoutSignal) -> LayoutWatch {
        let started = self.0.as_ref().and_then(|shared| {
            let inbox = shared.inbox.borrow_mut().take()?;
            let watcher = Watcher::new(Rc::clone(&shared.session), feed, changed);
            Some(watcher.run(inbox))
        });
        match started {
            Some(watcher) => Box::pin(watcher),
            None => Box::pin(pending()),
        }
    }

    /// At exit, however the daemon ends once it holds its name: unloads both
    /// scripts and removes their files, within two calls' deadlines (asking
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
