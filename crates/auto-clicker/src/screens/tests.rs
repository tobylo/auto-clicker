use std::cell::{Cell, RefCell};
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::task::{Context, Poll, Waker};
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
    /// The layout its watcher reports when next polled.
    staged: RefCell<Option<Option<Layout>>>,
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
        Box::pin(poll_fn(move |_| {
            let staged = source.0.staged.take();
            if let Some(layout) = staged {
                source.set(layout);
            }
            if source.0.ends.get() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }))
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
            assert!(fired(&changed).await);
            wayland.set(wide(2048));
            settle().await;
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

/// Watches `kwin` and `wayland`, lets `change` act on them once the
/// watcher waits, and says whether the controller was told.
async fn told_after(kwin: &Fake, wayland: &Fake, change: impl FnOnce()) -> bool {
    let local = LocalSet::new();
    local
        .run_until(async {
            let screens = Screens::new(kwin.clone(), wayland.clone());
            let changed = watched(&local, &screens);
            settle().await;
            change();
            settle().await;
            fired(&changed).await
        })
        .await
}

#[tokio::test(start_paused = true)]
async fn a_source_whose_watcher_ends_leaves_the_other_watched() {
    let (kwin, wayland) = (Fake::default(), Fake::default());
    wayland.0.ends.set(true);
    assert!(told_after(&kwin, &wayland, || kwin.set(wide(2048))).await);
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
            wayland.0.staged.replace(Some(wide(2560)));
            let changed = watched(&local, &screens);
            settle().await;
            assert!(fired(&changed).await);
            assert_eq!(screens.layout(), wide(2560));
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn a_change_undone_before_it_was_read_still_fires() {
    let (kwin, wayland) = (Fake::default(), Fake::default());
    kwin.set(wide(1920));
    // KWin read two reports in one poll: away and back.
    let away_and_back = || {
        kwin.set(wide(2048));
        kwin.set(wide(1920));
    };
    assert!(told_after(&kwin, &wayland, away_and_back).await);
}

/// Whether `changed` fired, polled once.
fn fired_now(changed: &LayoutSignal) -> bool {
    let mut cx = Context::from_waker(Waker::noop());
    pin!(changed.fired()).poll(&mut cx).is_ready()
}

#[test]
fn a_change_a_source_reads_reaches_the_controller_in_the_same_poll() {
    let (kwin, wayland) = (Fake::default(), Fake::default());
    let screens = Screens::new(kwin.clone(), wayland.clone());
    let changed = LayoutSignal::default();
    let mut watch = screens.watch(RecordingSlot::new().feed(), changed.clone());
    let mut cx = Context::from_waker(Waker::noop());
    // More polls than branches: each branch has its turn to go first in
    // an unbiased join.
    for width in [1920, 2048, 2560, 3440] {
        kwin.0.staged.replace(Some(wide(width)));
        assert!(watch.as_mut().poll(&mut cx).is_pending());
        assert!(fired_now(&changed), "the change to {width} waited a poll");
    }
    kwin.set(None);
    assert!(watch.as_mut().poll(&mut cx).is_pending());
    assert!(fired_now(&changed));
    for width in [1280, 1366, 1440] {
        wayland.0.staged.replace(Some(wide(width)));
        assert!(watch.as_mut().poll(&mut cx).is_pending());
        assert!(
            fired_now(&changed),
            "the display's change to {width} waited a poll"
        );
    }
}
