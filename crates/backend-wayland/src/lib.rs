//! The screen layout from xdg-output, on any Wayland desktop that offers it
//! (W01, decision 0004).
//!
//! The daemon connects to its Wayland display once at start
//! ([`Wayland::connect`]), after it holds its name. The source is picked by
//! what answers: a display offering `zxdg_output_manager_v1` gives the
//! layout, any other gives none, with the reason kept ([`Missing`]). Nothing
//! outside this crate talks to the display. A lost connection or a protocol
//! error makes the layout unknown for the daemon's life and never ends the
//! daemon (brief W5).
//!
//! The process listening on the display's socket ([`Wayland::compositor`])
//! is kept whatever happens to the connection: it tells KWin's backend
//! whether KWin is this display's compositor (brief W1).

mod client;
#[cfg(test)]
mod compositor_tests;
mod gather;
mod socket;

use std::cell::{Cell, RefCell};
use std::future::pending;
use std::num::NonZeroU32;
use std::rc::Rc;

use auto_clicker_core::controller::{LayoutSignal, LayoutWatch};
use auto_clicker_core::recording::Layout;

use client::Link;

/// Why the layout does not come from xdg-output, as W01e will word it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    /// The daemon was started without `WAYLAND_DISPLAY`.
    NoDisplay,
    /// The display did not answer, or not in time.
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
    /// The connection, until the watcher takes it.
    link: RefCell<Option<Link>>,
}

impl Wayland {
    /// Connects to the display `WAYLAND_DISPLAY` names and reads the first
    /// layout. The caller bounds the wait.
    pub async fn connect() -> Wayland {
        let Some(path) = socket::environment_path() else {
            eprintln!("auto-clicker: no Wayland display, so the screens are unknown");
            return Wayland::none(Missing::NoDisplay);
        };
        let stream = match socket::open(&path).await {
            Ok(stream) => stream,
            Err(missing) => return Wayland::none(missing),
        };
        let compositor = socket::listener(&stream);
        match Link::start(stream).await {
            Ok((link, layout)) => Wayland::of(compositor, layout, Ok(link)),
            Err(missing) => {
                if missing == Missing::NoXdgOutput {
                    eprintln!(
                        "auto-clicker: the desktop offers no xdg-output, so the screens are unknown"
                    );
                }
                Wayland::of(compositor, None, Err(missing))
            }
        }
    }

    /// No display: the layout is never known, for `missing`.
    pub fn none(missing: Missing) -> Wayland {
        Wayland::of(None, None, Err(missing))
    }

    fn of(
        compositor: Option<NonZeroU32>,
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
            link: RefCell::new(link),
        }))
    }

    /// The current layout, moved to start at (0, 0); `None` while it is not
    /// known.
    pub fn layout(&self) -> Option<Layout> {
        self.0.layout.borrow().clone()
    }

    /// Why there is no layout from xdg-output for good; `None` while the
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
    /// unknown. Only the first call gets it; any later one, like one
    /// without a connection, watches nothing.
    pub fn watch(&self, changed: LayoutSignal) -> LayoutWatch {
        let link = self.0.link.borrow_mut().take();
        match link {
            Some(link) => Box::pin(watch(Rc::clone(&self.0), link, changed)),
            None => Box::pin(pending()),
        }
    }
}

/// Reads the connection until it fails, then drops it: the layout is
/// unknown from then on.
async fn watch(shared: Rc<Shared>, mut link: Link, changed: LayoutSignal) {
    let ended = loop {
        if let Err(missing) = link.dispatch() {
            break missing;
        }
        if let Some(layout) = link.client.settled.take() {
            shared.publish(layout, &changed);
        }
        if let Err(missing) = link.wait().await {
            break missing;
        }
    };
    drop(link);
    shared.missing.set(Some(ended));
    shared.publish(None, &changed);
}

impl Shared {
    /// Makes `layout` the current one, and tells the controller when that
    /// changed it.
    fn publish(&self, layout: Option<Layout>, changed: &LayoutSignal) {
        let old = self.layout.replace(layout);
        if *self.layout.borrow() != old {
            changed.changed();
        }
    }
}
