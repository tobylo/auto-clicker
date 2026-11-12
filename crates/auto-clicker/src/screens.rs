//! The screen layout as the platforms give it: KWin's while KWin reports
//! one, xdg-output's otherwise (W01 brief, W1, until W01b).
//!
//! Each source writes only its own layout; this reads them and is the one
//! thing the controller polls. Its watcher runs both sources' watchers and
//! fires the controller's signal only when the layout read here changed:
//! on Plasma both sources see one change, and every fire refuses a pick
//! begun before it.

use std::rc::Rc;

use auto_clicker_backend_kwin::Kwin;
use auto_clicker_backend_wayland::Wayland;
use auto_clicker_core::controller::{LayoutSignal, LayoutWatch};
use auto_clicker_core::recording::{Layout, RecordingFeed};

/// A source of the layout.
pub trait Source {
    /// The layout now; `None` while it is not known.
    fn layout(&self) -> Option<Layout>;

    /// The source's watcher, firing `changed` when its layout changed.
    fn watch(&self, feed: RecordingFeed, changed: LayoutSignal) -> LayoutWatch;
}

impl Source for Kwin {
    fn layout(&self) -> Option<Layout> {
        Kwin::layout(self)
    }

    /// KWin's watcher also feeds a recording its positions.
    fn watch(&self, feed: RecordingFeed, changed: LayoutSignal) -> LayoutWatch {
        Kwin::watch(self, feed, changed)
    }
}

impl Source for Wayland {
    fn layout(&self) -> Option<Layout> {
        Wayland::layout(self)
    }

    fn watch(&self, _feed: RecordingFeed, changed: LayoutSignal) -> LayoutWatch {
        Wayland::watch(self, changed)
    }
}

/// KWin and the Wayland display, read as one layout.
pub struct Screens<K = Kwin, W = Wayland>(Rc<Both<K, W>>);

struct Both<K, W> {
    kwin: K,
    wayland: W,
}

impl<K, W> Clone for Screens<K, W> {
    fn clone(&self) -> Self {
        Screens(Rc::clone(&self.0))
    }
}

impl<K: Source + 'static, W: Source + 'static> Screens<K, W> {
    pub fn new(kwin: K, wayland: W) -> Self {
        Screens(Rc::new(Both { kwin, wayland }))
    }

    /// KWin's layout while it reports one, the display's otherwise.
    pub fn layout(&self) -> Option<Layout> {
        self.0.kwin.layout().or_else(|| self.0.wayland.layout())
    }

    /// Runs both sources' watchers, and fires `changed` each time the
    /// layout [`Screens::layout`] reads changed. A source whose watcher
    /// ends leaves the other running. Changes count from the layout when
    /// the watcher is made, not when it is first polled: a source may
    /// report one on its own first poll, before the forwarding begins.
    pub fn watch(&self, feed: RecordingFeed, changed: LayoutSignal) -> LayoutWatch {
        let heard = LayoutSignal::default();
        let kwin = self.0.kwin.watch(feed.clone(), heard.clone());
        let wayland = self.0.wayland.watch(feed, heard.clone());
        let screens = self.clone();
        let mut last = self.layout();
        Box::pin(async move {
            let forward = async {
                loop {
                    heard.fired().await;
                    let now = screens.layout();
                    if now != last {
                        last = now;
                        changed.changed();
                    }
                }
            };
            tokio::join!(kwin, wayland, forward);
        })
    }
}

#[cfg(test)]
mod tests;
