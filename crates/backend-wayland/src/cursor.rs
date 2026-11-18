//! The pointer's position in recordings from the display's cursor
//! sessions (ext-image-copy-capture), as niri gives it (W02, decision
//! 0004 "niri cursor sessions").
//!
//! - **Found by what answers (W1).** The layout connection notes at start
//!   whether the display offers a seat and both managers; nothing is
//!   bound then, and nothing is asked of the display while no recording
//!   runs (W7). Whether a recording uses them is asked at its start
//!   ([`Cursors::watch`]'s `open`): the platform says no when KWin reports
//!   the pointer or Hyprland was found, whose own routes are measured.
//! - **One connection per recording.** Each recording opens its own, to
//!   the listener found at start, binds the outputs and reads their layout,
//!   then opens a cursor session per output, and one more for each output
//!   plugged in meanwhile. Its stop, or a new recording, drops it and with
//!   it every object it made; the layout connection is never touched, so a
//!   failure here costs positions only (W5, W6).
//! - **Bounded.** The setup has [`SETUP`] to finish; after it, the
//!   connection is read until the recording ends.
//! - **The layout (W4).** Positions and the outputs' updates come in order
//!   on the one connection: a position heard while an update is not
//!   settled is dropped, and every other is placed on the layout settled
//!   last ([`Gather::position`]) and handed to the recorder with it.
//! - **The reason.** A setup that fails or runs late, a connection that
//!   ends, and a seat that loses its pointer set
//!   [`NoPointer::DisplayNoPosition`] for the rest of the recording; its
//!   stop clears it.
//!
//! [`Gather::position`]: crate::gather::Gather::position

use std::cell::{Cell, RefCell};
use std::future::{Future, pending};
use std::rc::Rc;
use std::time::Duration;

use auto_clicker_core::controller::{LayoutSignal, LayoutWatch};
use auto_clicker_core::message::NoPointer;
use auto_clicker_core::recording::{Layout, Point, PointerSource, RecordingFeed, Taken};
use tokio::time::{Instant, timeout_at};

use crate::client::{Link, Refused};
use crate::{Shared, Wayland};

/// How long a recording's connection has to be set up: connected, its
/// outputs' layout read, the seat's capabilities known and the sessions
/// asked for. Well past a round trip on a busy desktop.
pub(crate) const SETUP: Duration = Duration::from_secs(1);

/// The display's cursor sessions as the daemon holds them. Clones share
/// them.
#[derive(Clone)]
pub struct Cursors(Rc<Inner>);

struct Inner {
    display: Rc<Shared>,
    reason: Reason,
    clock: fn() -> Duration,
    watched: Cell<bool>,
}

impl Cursors {
    /// The cursor sessions of `wayland`'s display; `clock` stamps positions
    /// (the event clock).
    pub fn of(wayland: &Wayland, clock: fn() -> Duration) -> Cursors {
        Cursors(Rc::new(Inner {
            display: Rc::clone(&wayland.0),
            reason: Reason::default(),
            clock,
            watched: Cell::new(false),
        }))
    }

    /// Whether the display offered cursor sessions at start, and if so
    /// why the running recording gets no position from them, if it does
    /// not; `None` when it did not offer them.
    pub fn found(&self) -> Option<Result<(), NoPointer>> {
        self.0.display.cursors.then(|| self.0.reason.get())
    }

    /// The watcher, to be polled on the daemon's thread: for each recording
    /// `feed` starts while `open` holds, it opens a connection with cursor
    /// sessions and feeds the recording its positions, and it fires
    /// `changed` when [`Cursors::found`] changes. Only the first call on a
    /// display that offered them gets it; any other watches nothing.
    pub fn watch(
        &self,
        feed: RecordingFeed,
        open: Box<dyn Fn() -> bool>,
        changed: LayoutSignal,
    ) -> LayoutWatch {
        if !self.0.display.cursors || self.0.watched.replace(true) {
            return Box::pin(pending());
        }
        self.0.reason.tell(changed);
        let watcher = Watcher {
            inner: Rc::clone(&self.0),
            feed,
            open,
        };
        Box::pin(watcher.run())
    }
}

/// Why the running recording gets no position, if it does not; a change
/// fires the layout signal.
#[derive(Default)]
struct Reason {
    now: Cell<Option<NoPointer>>,
    changed: RefCell<Option<LayoutSignal>>,
}

impl Reason {
    fn get(&self) -> Result<(), NoPointer> {
        self.now.get().map_or(Ok(()), Err)
    }

    fn set(&self, reason: Option<NoPointer>) {
        if self.now.replace(reason) == reason {
            return;
        }
        if let Some(why) = reason {
            eprintln!("auto-clicker: this recording gets no positions: {why}");
        }
        if let Some(signal) = &*self.changed.borrow() {
            signal.changed();
        }
    }

    fn tell(&self, changed: LayoutSignal) {
        self.changed.replace(Some(changed));
    }
}

struct Watcher {
    inner: Rc<Inner>,
    feed: RecordingFeed,
    open: Box<dyn Fn() -> bool>,
}

impl Watcher {
    /// Waits for each start or stop of a recording; whatever the last one
    /// had under way is dropped then, and a recording that runs gets a new
    /// connection. A stop and a start told at once are a new recording.
    async fn run(self) {
        let mut changes = self.feed.clone();
        loop {
            let round = (self.feed.recording() && (self.open)()).then(|| self.round());
            tokio::select! {
                biased;
                () = changes.changed() => {}
                () = maybe(round) => {}
            }
            self.inner.reason.set(None);
        }
    }

    /// One recording's connection, until it ends; then why, until the
    /// recording does.
    async fn round(&self) {
        let deadline = Instant::now() + SETUP;
        let opened = match timeout_at(deadline, self.open()).await {
            Ok(opened) => opened,
            Err(_) => {
                eprintln!(
                    "auto-clicker: the Wayland display did not set up the pointer's cursor \
                     sessions within {SETUP:?}"
                );
                None
            }
        };
        if let Some(link) = opened {
            self.read(link).await;
        }
        self.inner.reason.set(Some(NoPointer::DisplayNoPosition));
        pending::<()>().await;
    }

    /// A connection to the display found at start, its first layout read
    /// and a cursor session on each output; `None` when that fails.
    async fn open(&self) -> Option<Link> {
        let stream = self.inner.display.open_checked().await.ok()?;
        let (mut link, _) = Link::start(stream).await.ok()?;
        match link.open_sessions().await.ok()? {
            Ok(()) => Some(link),
            Err(Refused::NotOffered) => {
                eprintln!("auto-clicker: the Wayland display no longer offers cursor sessions");
                None
            }
            Err(Refused::NoPointer) => {
                eprintln!("auto-clicker: the Wayland display's seat has no pointer");
                None
            }
        }
    }

    /// Reads `link` until it ends or its seat loses the pointer, whose
    /// sessions then report nothing more, handing each position placed to
    /// the recording, stamped when it was read.
    async fn read(&self, mut link: Link) {
        loop {
            if link.dispatch().is_err() {
                return;
            }
            if !link.client.has_pointer() {
                eprintln!("auto-clicker: the Wayland display's seat no longer has a pointer");
                return;
            }
            link.client.settled = None;
            self.feed_placed(std::mem::take(&mut link.client.positions));
            if link.wait().await.is_err() {
                return;
            }
        }
    }

    /// Hands `placed` to the recording, stamped now.
    fn feed_placed(&self, placed: Vec<(Point, Layout)>) {
        if placed.is_empty() {
            return;
        }
        let stamp = (self.inner.clock)();
        self.feed.with(|recorder| {
            for (point, layout) in &placed {
                let taken = Taken {
                    by: PointerSource::CursorSessions,
                    on: Some(layout),
                };
                recorder.position(stamp, *point, taken);
            }
        });
    }
}

/// `round` until it ends; never, without one.
async fn maybe(round: Option<impl Future<Output = ()>>) {
    match round {
        Some(round) => round.await,
        None => pending().await,
    }
}

#[cfg(test)]
mod tests;
