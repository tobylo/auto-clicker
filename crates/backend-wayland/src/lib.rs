//! The screen layout from xdg-output, on any Wayland desktop that offers it
//! (W01, decision 0004).
//!
//! The daemon connects to its Wayland display at start ([`Wayland::connect`]),
//! after it holds its name, within a deadline. The source is picked by what
//! answers: a display offering `zxdg_output_manager_v1` gives the layout,
//! any other gives none, with the reason kept ([`NoScreens`]). Nothing outside
//! this crate talks to the display. A lost connection or a protocol error
//! makes the layout unknown and never ends the daemon (brief W5).
//!
//! The process listening on the display's socket ([`Wayland::compositor`])
//! is kept whatever happens to the connection: it tells KWin's backend
//! whether KWin is this display's compositor (brief W1). On Plasma that
//! listener, `kwin_wayland_wrapper`, keeps the socket while KWin restarts
//! under it: each time the daemon hears of a KWin of the display, the
//! first one included, the watcher connects again to the same listener,
//! with no deadline, and the layout comes back (W5). Elsewhere nothing asks it to, and a lost
//! connection leaves the layout unknown while the session lasts.

mod client;
#[cfg(test)]
mod compositor_tests;
mod gather;
pub mod process;
mod socket;

use std::cell::{Cell, RefCell};
use std::future::{Future, pending, poll_fn};
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::pin::pin;
use std::rc::Rc;
use std::task::Poll;

use auto_clicker_core::controller::{LayoutSignal, LayoutWatch};
use auto_clicker_core::message::NoScreens;
use auto_clicker_core::recording::Layout;
use tokio::time::{Instant, timeout_at};

use client::Link;
pub use gather::Corner;
use gather::Settled;

/// The Wayland display as the daemon uses it. Clones share one connection.
#[derive(Clone)]
pub struct Wayland(Rc<Shared>);

struct Shared {
    /// The last whole layout, or why it is not known.
    layout: RefCell<Result<Layout, NoScreens>>,
    /// Where the last whole layout was moved from.
    corner: Cell<Corner>,
    /// How many times the layout changed, became unknown or is unknown for
    /// another reason: each time the controller's signal fires.
    changes: Cell<u64>,
    compositor: Option<NonZeroU32>,
    /// The display's socket, as found at start: where a connection is made
    /// again. `None` without a display.
    path: Option<PathBuf>,
    /// The connection, until the watcher takes it.
    link: RefCell<Option<Link>>,
    /// A watcher was handed out.
    watched: Cell<bool>,
}

impl Wayland {
    /// Connects to the display `WAYLAND_DISPLAY` names and reads the first
    /// layout, giving up at `deadline`. A display that answers too late
    /// keeps its listener, so KWin is still checked against it.
    pub async fn connect(deadline: Instant) -> Wayland {
        let Some(path) = socket::environment_path() else {
            eprintln!("auto-clicker: no Wayland display, so the screens are unknown");
            return Wayland::none(NoScreens::NoDisplay);
        };
        let opened = match timeout_at(deadline, socket::open(&path)).await {
            Ok(opened) => opened,
            Err(_) => Err(too_late()),
        };
        match opened {
            Ok(stream) => Wayland::over(stream, deadline, path).await,
            Err(missing) => Wayland::of(None, Some(path), Settled::default(), Err(missing)),
        }
    }

    /// The display on the other end of `stream`, at `path`, its first
    /// layout read by `deadline`.
    async fn over(stream: UnixStream, deadline: Instant, path: PathBuf) -> Wayland {
        let compositor = socket::listener(&stream);
        let started = match timeout_at(deadline, Link::start(stream)).await {
            Ok(started) => started,
            Err(_) => Err(too_late()),
        };
        let path = Some(path);
        match started {
            Ok((link, first)) => Wayland::of(compositor, path, first, Ok(link)),
            Err(missing) => Wayland::of(compositor, path, Settled::default(), Err(said(missing))),
        }
    }

    /// No display: the layout is never known, for `why`.
    pub fn none(why: NoScreens) -> Wayland {
        Wayland::of(None, None, Settled::default(), Err(why))
    }

    fn of(
        compositor: Option<NonZeroU32>,
        path: Option<PathBuf>,
        first: Settled,
        link: Result<Link, NoScreens>,
    ) -> Wayland {
        let (link, layout) = match link {
            Ok(link) => (Some(link), first.layout.ok_or(NoScreens::NotWhole)),
            Err(why) => (None, Err(why)),
        };
        Wayland(Rc::new(Shared {
            layout: RefCell::new(layout),
            corner: Cell::new(first.corner),
            changes: Cell::new(0),
            compositor,
            path,
            link: RefCell::new(link),
            watched: Cell::new(false),
        }))
    }

    /// The current layout, moved to start at (0, 0), or why it is not
    /// known.
    pub fn layout(&self) -> Result<Layout, NoScreens> {
        self.0.layout.borrow().clone()
    }

    /// The current layout, where it was moved from, and how many times it
    /// changed since the start: one value, so a position is never moved by
    /// one layout's corner and judged by another's (brief W4).
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            layout: self.layout(),
            corner: self.0.corner.get(),
            changes: self.changes(),
        }
    }

    /// How many times the layout changed, became unknown or is unknown for
    /// another reason since the start; the same count [`Wayland::snapshot`]
    /// gives.
    pub fn changes(&self) -> u64 {
        self.0.changes.get()
    }

    /// The process listening on the display's socket; `None` without a
    /// display, or when it cannot be told.
    pub fn compositor(&self) -> Option<NonZeroU32> {
        self.0.compositor
    }

    /// The watcher, to be polled on the daemon's thread: it reads the
    /// connection and fires `changed` when the layout changes, becomes
    /// unknown, or is unknown for another reason. Each time `again` fires
    /// (a new KWin of the display), it drops the connection, makes a new
    /// one to the same listener and reads its first layout, with no
    /// deadline; a later fire starts that over.
    /// Only the first call gets it; any later one watches nothing. Without
    /// a display it never connects, and the layout stays unknown.
    pub fn watch(&self, changed: LayoutSignal, again: LayoutSignal) -> LayoutWatch {
        if self.0.watched.replace(true) {
            return Box::pin(pending());
        }
        let link = self.0.link.borrow_mut().take();
        Box::pin(watch(Rc::clone(&self.0), link, changed, again))
    }
}

/// The layout at one moment ([`Wayland::snapshot`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub layout: Result<Layout, NoScreens>,
    /// Where the layout was moved from; meaningless while it is unknown.
    pub corner: Corner,
    /// The change count at that moment ([`Wayland::changes`]).
    pub changes: u64,
}

fn too_late() -> NoScreens {
    eprintln!("auto-clicker: the Wayland display did not answer, so the screens are unknown");
    NoScreens::CannotConnect
}

/// `missing`, logged when it is the desktop's lack of xdg-output.
fn said(missing: NoScreens) -> NoScreens {
    if missing == NoScreens::NoXdgOutput {
        eprintln!("auto-clicker: the desktop offers no xdg-output, so the screens are unknown");
    }
    missing
}

/// Reads the connection until it ends, and makes a new one each time
/// `again` fires: the old one is dropped first, and the layout is unknown
/// until the new one gives a whole layout.
async fn watch(
    shared: Rc<Shared>,
    mut link: Option<Link>,
    changed: LayoutSignal,
    again: LayoutSignal,
) {
    loop {
        if let Some(running) = link.take() {
            if let Some(ended) = read_until_restart(&shared, running, &changed, &again).await {
                shared.end(ended, &changed);
                again.fired().await;
            }
        } else {
            again.fired().await;
        }
        link = shared.connect_again(&changed, &again).await;
    }
}

/// Reads `link` until it fails, and why; `None` once `again` fires first.
/// `again` is polled first, every time, and not through `select!`, which
/// returns at once when the task's budget is spent: a new KWin is heard,
/// and the layout made unknown, in the poll that told of it, whatever was
/// read before (`Notify` spends no budget). Both waits end with it: a wait
/// for `again` left behind would take the next fire.
async fn read_until_restart(
    shared: &Shared,
    link: Link,
    changed: &LayoutSignal,
    again: &LayoutSignal,
) -> Option<NoScreens> {
    let mut restart = pin!(again.fired());
    let mut reading = pin!(read(shared, link, changed));
    poll_fn(|cx| {
        if restart.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        reading.as_mut().poll(cx).map(Some)
    })
    .await
}

/// Reads `link` until it fails; why it did.
async fn read(shared: &Shared, mut link: Link, changed: &LayoutSignal) -> NoScreens {
    loop {
        if let Err(missing) = link.dispatch() {
            return missing;
        }
        if let Some(settled) = link.client.settled.take() {
            shared.publish(settled, changed);
        }
        if let Err(missing) = link.wait().await {
            return missing;
        }
    }
}

impl Shared {
    /// The layout is unknown, for `missing`.
    fn end(&self, missing: NoScreens, changed: &LayoutSignal) {
        self.set(Err(missing), false, changed);
    }

    /// A new connection to the display, its first layout published; `None`
    /// when it fails, with why kept. Each fire of `again` meanwhile starts
    /// it over: a newer KWin is what answers now.
    async fn connect_again(&self, changed: &LayoutSignal, again: &LayoutSignal) -> Option<Link> {
        loop {
            self.end(NoScreens::Lost, changed);
            tokio::select! {
                biased;
                () = again.fired() => {}
                started = self.start_again() => {
                    return match started {
                        Ok((link, first)) => {
                            self.publish(first, changed);
                            Some(link)
                        }
                        Err(missing) => {
                            self.end(said(missing), changed);
                            None
                        }
                    };
                }
            }
        }
    }

    /// Connects to the display's socket, checks the listener is still the
    /// one found at start, and reads the first layout.
    async fn start_again(&self) -> Result<(Link, Settled), NoScreens> {
        let path = self.path.as_ref().ok_or(NoScreens::NoDisplay)?;
        let stream = socket::open(path).await?;
        let listener = socket::listener(&stream);
        if self.compositor.is_some() && listener != self.compositor {
            eprintln!(
                "auto-clicker: another process listens on the Wayland display now, so the \
                 screens are unknown"
            );
            return Err(NoScreens::CannotConnect);
        }
        Link::start(stream).await
    }

    /// Makes `settled`'s layout the current one; a connection that gives
    /// none has not described every screen.
    fn publish(&self, settled: Settled, changed: &LayoutSignal) {
        let Settled {
            layout,
            corner,
            moved,
        } = settled;
        self.corner.set(corner);
        self.set(layout.ok_or(NoScreens::NotWhole), moved, changed);
    }

    /// Makes `layout` the current one, and tells the controller when a
    /// screen came, went, moved or changed size since the last (`moved`),
    /// the layout became known or unknown, or it is unknown for another
    /// reason. A new scale alone is kept, untold: stored, never compared
    /// (R03 decision 6).
    fn set(&self, layout: Result<Layout, NoScreens>, moved: bool, changed: &LayoutSignal) {
        let was = self.layout.replace(layout);
        let told = match (&was, &*self.layout.borrow()) {
            (Ok(_), Ok(_)) => moved,
            (was, now) => was != now,
        };
        if told {
            self.changes.set(self.changes.get().wrapping_add(1));
            changed.changed();
        }
    }
}
