//! The screen layout from xdg-output, on any Wayland desktop that offers it
//! (W01, decision 0004).
//!
//! The daemon connects to its Wayland display at start ([`Wayland::connect`]),
//! after it holds its name, within a deadline. The source is picked by what
//! answers: a display offering `zxdg_output_manager_v1` gives the layout,
//! any other gives none, with the reason kept ([`Missing`]). Nothing outside
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
mod socket;

use std::cell::{Cell, RefCell};
use std::future::pending;
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;

use auto_clicker_core::controller::{LayoutSignal, LayoutWatch};
use auto_clicker_core::recording::Layout;
use tokio::time::{Instant, timeout_at};

use client::Link;
use gather::Settled;

/// Why the layout does not come from xdg-output, as W01e will word it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    /// The daemon was started without `WAYLAND_DISPLAY`.
    NoDisplay,
    /// The display did not answer, or not in time, or another process than
    /// the one found at start listens on it now.
    CannotConnect,
    /// The compositor offers no xdg-output.
    NoXdgOutput,
    /// The connection closed or failed.
    Lost,
    /// The compositor or the client refused what the other sent.
    ProtocolError,
}

/// The Wayland display as the daemon uses it. Clones share one connection.
#[derive(Clone)]
pub struct Wayland(Rc<Shared>);

struct Shared {
    /// The last whole layout; `None` while it is not known.
    layout: RefCell<Option<Layout>>,
    missing: Cell<Option<Missing>>,
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
            return Wayland::none(Missing::NoDisplay);
        };
        let opened = match timeout_at(deadline, socket::open(&path)).await {
            Ok(opened) => opened,
            Err(_) => Err(too_late()),
        };
        match opened {
            Ok(stream) => Wayland::over(stream, deadline, path).await,
            Err(missing) => Wayland::of(None, Some(path), None, Err(missing)),
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
            Ok((link, layout)) => Wayland::of(compositor, path, layout, Ok(link)),
            Err(missing) => Wayland::of(compositor, path, None, Err(said(missing))),
        }
    }

    /// No display: the layout is never known, for `missing`.
    pub fn none(missing: Missing) -> Wayland {
        Wayland::of(None, None, None, Err(missing))
    }

    fn of(
        compositor: Option<NonZeroU32>,
        path: Option<PathBuf>,
        layout: Option<Layout>,
        link: Result<Link, Missing>,
    ) -> Wayland {
        let (link, missing) = match link {
            Ok(link) => (Some(link), None),
            Err(missing) => (None, Some(missing)),
        };
        Wayland(Rc::new(Shared {
            layout: RefCell::new(layout),
            missing: Cell::new(missing),
            compositor,
            path,
            link: RefCell::new(link),
            watched: Cell::new(false),
        }))
    }

    /// The current layout, moved to start at (0, 0); `None` while it is not
    /// known.
    pub fn layout(&self) -> Option<Layout> {
        self.0.layout.borrow().clone()
    }

    /// Why there is no layout from xdg-output now; `None` while a
    /// connection runs.
    pub fn missing(&self) -> Option<Missing> {
        self.0.missing.get()
    }

    /// The process listening on the display's socket; `None` without a
    /// display, or when it cannot be told.
    pub fn compositor(&self) -> Option<NonZeroU32> {
        self.0.compositor
    }

    /// The watcher, to be polled on the daemon's thread: it reads the
    /// connection and fires `changed` when the layout changes or becomes
    /// unknown. Each time `again` fires (a new KWin of the display), it
    /// drops the connection, makes a new one to the same listener and reads
    /// its first layout, with no deadline; a later fire starts that over.
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

fn too_late() -> Missing {
    eprintln!("auto-clicker: the Wayland display did not answer, so the screens are unknown");
    Missing::CannotConnect
}

/// `missing`, logged when it is the desktop's lack of xdg-output.
fn said(missing: Missing) -> Missing {
    if missing == Missing::NoXdgOutput {
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
            tokio::select! {
                biased;
                () = again.fired() => {}
                ended = read(&shared, running, &changed) => {
                    shared.end(ended, &changed);
                    again.fired().await;
                }
            }
        } else {
            again.fired().await;
        }
        link = shared.connect_again(&changed, &again).await;
    }
}

/// Reads `link` until it fails; why it did.
async fn read(shared: &Shared, mut link: Link, changed: &LayoutSignal) -> Missing {
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
    fn end(&self, missing: Missing, changed: &LayoutSignal) {
        self.missing.set(Some(missing));
        let gone = Settled {
            layout: None,
            moved: false,
        };
        self.publish(gone, changed);
    }

    /// A new connection to the display, its first layout published; `None`
    /// when it fails, with why kept. Each fire of `again` meanwhile starts
    /// it over: a newer KWin is what answers now.
    async fn connect_again(&self, changed: &LayoutSignal, again: &LayoutSignal) -> Option<Link> {
        loop {
            self.end(Missing::Lost, changed);
            tokio::select! {
                biased;
                () = again.fired() => {}
                started = self.start_again() => {
                    return match started {
                        Ok((link, layout)) => {
                            self.missing.set(None);
                            let first = Settled { layout, moved: false };
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
    async fn start_again(&self) -> Result<(Link, Option<Layout>), Missing> {
        let path = self.path.as_ref().ok_or(Missing::NoDisplay)?;
        let stream = socket::open(path).await?;
        let listener = socket::listener(&stream);
        if self.compositor.is_some() && listener != self.compositor {
            eprintln!(
                "auto-clicker: another process listens on the Wayland display now, so the \
                 screens are unknown"
            );
            return Err(Missing::CannotConnect);
        }
        Link::start(stream).await
    }

    /// Makes `settled`'s layout the current one, and tells the controller
    /// when a screen came, went, moved or changed size since the last, or
    /// the layout became known or unknown. A new scale alone is kept,
    /// untold: stored, never compared (R03 decision 6).
    fn publish(&self, settled: Settled, changed: &LayoutSignal) {
        let Settled { layout, moved } = settled;
        let was_known = self.layout.replace(layout).is_some();
        if moved || was_known != self.layout.borrow().is_some() {
            changed.changed();
        }
    }
}
