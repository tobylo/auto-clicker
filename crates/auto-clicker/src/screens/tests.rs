use std::cell::{Cell, RefCell};
use std::future::pending;
use std::time::Duration;

use auto_clicker_core::recording::{Output, RecordingSlot, Rect};
use tokio::task::{LocalSet, yield_now};
use tokio::time::timeout;

use super::*;

/// A source whose layout a test sets, and whose watcher it ends.
#[derive(Clone, Default)]
struct Fake(Rc<FakeState>);

#[derive(Default)]
struct FakeState {
    layout: RefCell<Option<Layout>>,
    signal: RefCell<Option<LayoutSignal>>,
    /// Its watcher ends at once.
    ends: Cell<bool>,
    /// The layout its watcher reports on its first poll.
    at_start: RefCell<Option<Option<Layout>>>,
}

impl Fake {
    /// The source's layout is `layout` now, and it says so.
    fn set(&self, layout: Option<Layout>) {
        self.0.layout.replace(layout);
        if let Some(signal) = &*self.0.signal.borrow() {
            signal.changed();
        }
    }
}

impl Source for Fake {
    fn layout(&self) -> Option<Layout> {
        self.0.layout.borrow().clone()
    }

    fn watch(&self, _feed: RecordingFeed, changed: LayoutSignal) -> LayoutWatch {
        self.0.signal.replace(Some(changed));
        let source = self.clone();
        Box::pin(async move {
            let at_start = source.0.at_start.take();
            if let Some(layout) = at_start {
                source.set(layout);
            }
            if !source.0.ends.get() {
                pending::<()>().await;
            }
        })
    }
}

fn wide(width: u32) -> Option<Layout> {
    let rect = Rect {
        x: 0,
        y: 0,
        width,
        height: 1080,
    };
    Some(Layout::new(vec![Output::new(rect, 1000).unwrap()]).unwrap())
}

async fn settle() {
    for _ in 0..16 {
        yield_now().await;
    }
}

/// Whether `changed` fired since last asked.
async fn fired(changed: &LayoutSignal) -> bool {
    timeout(Duration::ZERO, changed.fired()).await.is_ok()
}

/// Watches `screens` on `local`; the controller's signal.
fn watched(local: &LocalSet, screens: &Screens<Fake, Fake>) -> LayoutSignal {
    let changed = LayoutSignal::default();
    local.spawn_local(screens.watch(RecordingSlot::new().feed(), changed.clone()));
    changed
}

#[test]
fn kwins_layout_wins_while_it_has_one() {
    let (kwin, wayland) = (Fake::default(), Fake::default());
    let screens = Screens::new(kwin.clone(), wayland.clone());
    assert_eq!(screens.layout(), None);
    wayland.set(wide(1920));
    assert_eq!(screens.layout(), wide(1920));
    kwin.set(wide(2048));
    assert_eq!(screens.layout(), wide(2048));
    wayland.set(None);
    assert_eq!(screens.layout(), wide(2048));
    kwin.set(None);
    assert_eq!(screens.layout(), None);
}

#[tokio::test(start_paused = true)]
async fn one_change_both_sources_see_fires_once() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (kwin, wayland) = (Fake::default(), Fake::default());
            kwin.set(wide(1920));
            wayland.set(wide(1920));
            let screens = Screens::new(kwin.clone(), wayland.clone());
            let changed = watched(&local, &screens);
            settle().await;
            kwin.set(wide(2048));
            settle().await;
            wayland.set(wide(2048));
            settle().await;
            assert!(fired(&changed).await);
            assert!(
                !fired(&changed).await,
                "the second source's report fired again"
            );
            // Only the display changed, while KWin's layout stands.
            wayland.set(wide(1280));
            settle().await;
            assert!(!fired(&changed).await);
            // KWin's layout goes: the display's is the layout now.
            kwin.set(None);
            settle().await;
            assert!(fired(&changed).await);
            assert_eq!(screens.layout(), wide(1280));
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn a_source_whose_watcher_ends_leaves_the_other_watched() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (kwin, wayland) = (Fake::default(), Fake::default());
            wayland.0.ends.set(true);
            let screens = Screens::new(kwin.clone(), wayland.clone());
            let changed = watched(&local, &screens);
            settle().await;
            kwin.set(wide(2048));
            settle().await;
            assert!(fired(&changed).await);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn a_change_a_source_reports_on_its_first_poll_fires() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (kwin, wayland) = (Fake::default(), Fake::default());
            wayland.set(wide(1920));
            let screens = Screens::new(kwin.clone(), wayland.clone());
            // The controller read the layout before the watcher first ran.
            assert_eq!(screens.layout(), wide(1920));
            wayland.0.at_start.replace(Some(wide(2560)));
            let changed = watched(&local, &screens);
            settle().await;
            assert!(fired(&changed).await);
            assert_eq!(screens.layout(), wide(2560));
        })
        .await;
}
