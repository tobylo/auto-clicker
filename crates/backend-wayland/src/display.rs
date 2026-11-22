//! The on-screen display: a tray start's countdown in the middle of every
//! screen, above the game (O01b, brief invariants O1 to O8).
//!
//! - **One more reader (O1).** What shows is the view's countdown, while
//!   `[on_screen] countdown` is on, on every screen: the display reads the
//!   view and nothing else, and tells the controller nothing. Whether the
//!   desktop can show it at all ([`OnScreen::found`]) is read from the
//!   globals the layout's connection heard when it last connected, and a
//!   change fires the layout's signal.
//! - **A connection of its own (O4), only while it shows (O5).** A countdown
//!   opens one, to the listener found at start, within [`SETUP`]; each
//!   screen's surface has [`ANSWER`] to be answered. When the number goes,
//!   the connection is dropped, and the compositor removes every surface
//!   with it: nothing waits on a removal (O2, O7), and nothing of the
//!   display stays while idle.
//! - **Never in the way (O2).** It is polled with the layout's watcher on
//!   the daemon's thread and never waits on the controller; a stop is seen
//!   between two screens' drawings.
//! - **Failures keep their meaning (O3).** A screen whose surface was
//!   closed, answered late or is past a bound shows nothing, with its reason,
//!   until the screens change or the next countdown; the others keep
//!   theirs. A failed or lost connection is tried again only when the
//!   layout's signal says the screens changed, or at the next countdown:
//!   never on a tick of the seconds, and never on a timer.

pub(crate) mod fit;
pub(crate) mod paint;
pub(crate) mod shm;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::future::pending;
use std::pin::pin;
use std::rc::Rc;
use std::time::Duration;

use auto_clicker_core::controller::{LayoutWatch, View};
use auto_clicker_core::digits::{self, Label};
use auto_clicker_core::message::{NoOnScreen, NoScreens};
use tokio::sync::watch;
use tokio::task::yield_now;
use tokio::time::{Instant, sleep_until, timeout_at};

use crate::client::{Link, Showing, Wanted};
use crate::{Shared, Wayland};

/// The namespace every surface carries, for compositor rules: the guide
/// names it, so a stream or a recording can leave the display out.
pub(crate) const NAMESPACE: &str = "auto-clicker-on-screen";

/// How long the display's connection has to be set up: connected, its
/// outputs read and its globals bound. Well past a round trip on a busy
/// desktop.
pub(crate) const SETUP: Duration = Duration::from_secs(1);

/// How long the compositor has to answer a surface's size.
pub(crate) const ANSWER: Duration = Duration::from_secs(1);

/// The on-screen display as the daemon holds it. Clones share it.
#[derive(Clone)]
pub struct OnScreen(Rc<Inner>);

struct Inner {
    display: Rc<Shared>,
    watched: Cell<bool>,
    /// Each screen as the running connection holds it; empty without one.
    screens: RefCell<BTreeMap<u32, Showing>>,
}

impl OnScreen {
    /// The on-screen display of `wayland`'s display.
    pub fn of(wayland: &Wayland) -> OnScreen {
        OnScreen(Rc::new(Inner {
            display: Rc::clone(&wayland.0),
            watched: Cell::new(false),
            screens: RefCell::new(BTreeMap::new()),
        }))
    }

    /// Whether the desktop can show the display, or why it cannot, as the
    /// layout's connection found when it last connected.
    pub fn found(&self) -> Result<(), NoOnScreen> {
        self.0.display.on_screen.get()
    }

    /// The watcher, to be polled on the daemon's thread: it shows what
    /// `views` says should show. Only the first call gets it; any later one
    /// shows nothing.
    pub fn watch(&self, views: watch::Receiver<View>) -> LayoutWatch {
        if self.0.watched.replace(true) {
            return Box::pin(pending());
        }
        let watcher = Watcher {
            inner: Rc::clone(&self.0),
            layout: self.0.display.layout_changes.subscribe(),
            views,
        };
        Box::pin(watcher.run())
    }

    /// Each screen as the display holds it now.
    #[cfg(test)]
    pub(crate) fn screens(&self) -> BTreeMap<u32, Showing> {
        self.0.screens.borrow().clone()
    }
}

/// Whether the desktop can show the display, from what the layout's
/// connection found: its globals, or why it did not connect.
pub(crate) fn found(link: Result<&Link, NoScreens>) -> Result<(), NoOnScreen> {
    match link {
        Ok(link) if link.client.display_globals.usable() => Ok(()),
        Ok(_) => Err(NoOnScreen::NoLayerShell),
        Err(NoScreens::NoDisplay) => Err(NoOnScreen::NoDisplay),
        Err(NoScreens::NoXdgOutput) => Err(NoOnScreen::NoXdgOutput),
        Err(_) => Err(NoOnScreen::CannotConnect),
    }
}

/// What should show while `view` holds: the countdown's label in the job's
/// colour while it is switched on; nothing otherwise, a hotkey's start
/// included, which waits for its keys and does not count down.
fn wanted(view: &View) -> Option<Wanted> {
    if !view.on_screen.countdown {
        return None;
    }
    let (seconds, colour) = digits::countdown(view.activity)?;
    Some(Wanted {
        label: Label::of(seconds),
        colour,
    })
}

/// Why a countdown's connection ended.
enum Ended {
    /// Nothing should show any more, or the controller is gone.
    Gone,
    /// It failed or was lost: the log says why.
    Failed,
}

struct Watcher {
    inner: Rc<Inner>,
    views: watch::Receiver<View>,
    /// The layout's changes: after a failed or lost connection, the signal
    /// that the screens changed.
    layout: watch::Receiver<u64>,
}

impl Watcher {
    /// Waits for something to show, shows it until it goes, and again.
    async fn run(mut self) {
        loop {
            let shows = self.wanted().is_some() && self.inner.display.on_screen.get().is_ok();
            if shows {
                self.countdown().await;
                self.inner.screens.borrow_mut().clear();
            }
            if self.views.changed().await.is_err() {
                return;
            }
        }
    }

    /// What should show now, the view marked as read.
    fn wanted(&mut self) -> Option<Wanted> {
        wanted(&self.views.borrow_and_update())
    }

    /// One countdown: a connection that shows it until nothing should; one
    /// that fails or is lost is made again only once the layout's signal
    /// says the screens changed after the failure (O3): a change heard
    /// before it was the connection's own to hear.
    async fn countdown(&mut self) {
        loop {
            let ended = match self.connect().await {
                Some(Ok(link)) => self.show(link).await,
                Some(Err(())) => Ended::Failed,
                None => Ended::Gone,
            };
            self.layout.borrow_and_update();
            self.inner.screens.borrow_mut().clear();
            if matches!(ended, Ended::Gone) || !self.screens_change().await {
                return;
            }
        }
    }

    /// The display's connection, set up within [`SETUP`]; `None` once
    /// nothing should show, which ends the setup at once.
    async fn connect(&mut self) -> Option<Result<Link, ()>> {
        let deadline = Instant::now() + SETUP;
        let display = Rc::clone(&self.inner.display);
        let mut setup = pin!(timeout_at(deadline, open(&display)));
        loop {
            tokio::select! {
                biased;
                changed = self.views.changed() => {
                    if changed.is_err() || self.wanted().is_none() {
                        return None;
                    }
                }
                opened = &mut setup => {
                    return Some(opened.unwrap_or_else(|_| {
                        eprintln!(
                            "auto-clicker: the Wayland display did not set up the countdown's \
                             surfaces within {SETUP:?}"
                        );
                        Err(())
                    }));
                }
            }
        }
    }

    /// Shows what should show on `link`'s screens until nothing should,
    /// or the connection fails. One screen is drawn per poll, the view read
    /// again between screens.
    async fn show(&mut self, mut link: Link) -> Ended {
        loop {
            let Some(wanted) = self.wanted() else {
                return Ended::Gone;
            };
            if link.dispatch().is_err() {
                return Ended::Failed;
            }
            link.client.settled = None;
            let drew = link.show(wanted, Instant::now());
            self.inner.screens.replace(link.showing());
            if drew {
                yield_now().await;
                continue;
            }
            let answer = link.next_answer();
            tokio::select! {
                biased;
                changed = self.views.changed() => {
                    if changed.is_err() {
                        return Ended::Gone;
                    }
                }
                waited = link.wait() => {
                    if waited.is_err() {
                        return Ended::Failed;
                    }
                }
                () = until(answer) => {}
            }
        }
    }

    /// Waits for the layout's signal; `false` once nothing should show
    /// first.
    async fn screens_change(&mut self) -> bool {
        loop {
            tokio::select! {
                biased;
                changed = self.views.changed() => {
                    if changed.is_err() || self.wanted().is_none() {
                        return false;
                    }
                }
                changed = self.layout.changed() => return changed.is_ok(),
            }
        }
    }
}

/// A connection to the display found at start, its outputs read and the
/// display's globals bound; `Err` when that fails, as the log says.
async fn open(display: &Shared) -> Result<Link, ()> {
    let stream = display.open_checked().await.map_err(drop)?;
    let (mut link, _) = Link::start(stream).await.map_err(drop)?;
    if !link.open_surfaces() {
        eprintln!("auto-clicker: the Wayland display no longer offers layer-shell");
        return Err(());
    }
    Ok(link)
}

/// Until `deadline`; never, without one.
async fn until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => pending().await,
    }
}

#[cfg(test)]
mod tests;
