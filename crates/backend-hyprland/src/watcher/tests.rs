//! The watcher against a scripted Hyprland and a layout the test moves: when
//! it asks, what reaches the recorder, and what the reason says.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use auto_clicker_backend_wayland::Corner;
use auto_clicker_core::controller::LayoutSignal;
use auto_clicker_core::hotkeys::Bindings;
use auto_clicker_core::message::{NoPointer, NoScreens};
use auto_clicker_core::recording::{
    Change, Created, Ending, Output, Recorder, RecordingSlot, Rect,
};
use tokio::task::{JoinHandle, LocalSet, yield_now};
use tokio::time::{advance, sleep, timeout};

use super::*;

thread_local! {
    /// When the event clock reads one second.
    static EPOCH: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The event clock: the test runtime's paused time, from one second.
fn clock() -> Duration {
    let epoch = EPOCH.get().unwrap_or_else(|| {
        let now = Instant::now();
        EPOCH.set(Some(now));
        now
    });
    Duration::from_secs(1) + epoch.elapsed()
}

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn us(us: u64) -> Duration {
    Duration::from_micros(us)
}

fn screens(width: u32) -> Layout {
    let rect = Rect {
        x: 0,
        y: 0,
        width,
        height: 1080,
    };
    Layout::new(vec![Output::new(rect, 1000).unwrap()]).unwrap()
}

/// The display's layout, which a test changes, and its change count.
#[derive(Clone)]
struct FakeScreens(Rc<RefCell<(Snapshot, u64)>>);

impl FakeScreens {
    fn new(corner: Corner) -> FakeScreens {
        let now = Snapshot {
            layout: Ok(screens(1920)),
            corner,
            changes: Some(0),
        };
        FakeScreens(Rc::new(RefCell::new((now, 0))))
    }

    /// A layout settles: the count moves.
    fn change(&self, layout: Result<Layout, NoScreens>) {
        let (now, count) = &mut *self.0.borrow_mut();
        *count += 1;
        now.layout = layout;
        now.changes = Some(*count);
    }

    /// The display tells of a change no layout settled yet.
    fn hear(&self) {
        self.0.borrow_mut().0.changes = None;
    }
}

impl Screens for FakeScreens {
    fn snapshot(&self) -> Snapshot {
        self.0.borrow().0.clone()
    }

    fn changes(&self) -> Option<u64> {
        self.0.borrow().0.changes
    }
}

/// How the scripted Hyprland answers an ask.
#[derive(Clone, Copy)]
enum Reply {
    After(Duration, Result<Global, Failure>),
    Never,
}

const HERE: Global = Global { x: 100, y: 200 };

#[derive(Default)]
struct Script {
    /// When each ask went out.
    asked: Vec<Instant>,
    /// Answers in turn; then [`HERE`] at once.
    replies: VecDeque<Reply>,
    /// Asks dropped before they were answered.
    dropped: usize,
}

#[derive(Clone, Default)]
struct Scripted(Rc<RefCell<Script>>);

/// Counts an ask dropped unanswered.
struct Dropped(Rc<RefCell<Script>>, bool);

impl Dropped {
    fn answered(&mut self) {
        self.1 = true;
    }
}

impl Drop for Dropped {
    fn drop(&mut self) {
        if !self.1 {
            self.0.borrow_mut().dropped += 1;
        }
    }
}

impl Asker for Scripted {
    fn ask(&self) -> Asking {
        let mut script = self.0.borrow_mut();
        script.asked.push(Instant::now());
        let reply = script
            .replies
            .pop_front()
            .unwrap_or(Reply::After(Duration::ZERO, Ok(HERE)));
        let mut guard = Dropped(Rc::clone(&self.0), false);
        Box::pin(async move {
            match reply {
                Reply::After(wait, answer) => {
                    sleep(wait).await;
                    guard.answered();
                    answer
                }
                Reply::Never => pending().await,
            }
        })
    }
}

impl Scripted {
    fn then(&self, reply: Reply) {
        self.0.borrow_mut().replies.push_back(reply);
    }

    fn asked(&self) -> Vec<Instant> {
        self.0.borrow().asked.clone()
    }

    fn dropped(&self) -> usize {
        self.0.borrow().dropped
    }
}

struct Harness {
    slot: RecordingSlot,
    feed: RecordingFeed,
    screens: FakeScreens,
    asker: Scripted,
    reason: Reason,
    changed: LayoutSignal,
    _watch: JoinHandle<()>,
}

impl Harness {
    fn new(local: &LocalSet, corner: Corner) -> Harness {
        let slot = RecordingSlot::new();
        let screens = FakeScreens::new(corner);
        let asker = Scripted::default();
        let reason = Reason::default();
        let changed = LayoutSignal::default();
        reason.tell(changed.clone());
        let watcher = Watcher {
            screens: screens.clone(),
            asker: asker.clone(),
            feed: slot.feed(),
            reason: reason.clone(),
            clock,
        };
        Harness {
            feed: slot.feed(),
            slot,
            screens,
            asker,
            reason,
            changed,
            _watch: local.spawn_local(watcher.run()),
        }
    }

    async fn record(&self) {
        let created = Created::parse("2026-11-13T12:00:00.000+01:00").unwrap();
        let recorder = Recorder::new(Bindings::default(), clock(), created, None);
        self.slot.start(recorder.with_layout(screens(1920)));
        settle().await;
    }

    async fn moved(&self) {
        self.feed.motion_read(clock(), 1, 0);
        settle().await;
    }

    /// Stops the recording, with a wheel turn so it is saved; its
    /// positions, each with its time.
    async fn stop(&self) -> Vec<(u64, Point)> {
        let mut recorder = self.slot.take().unwrap();
        settle().await;
        recorder.wheel(clock(), 1);
        let recording = recorder.finish(Ending::At(clock())).unwrap().unwrap();
        if recording.layout().is_some() {
            assert_eq!(recording.positions_from(), Some(PointerSource::Hyprland));
        }
        recording
            .events()
            .iter()
            .filter_map(|event| match event.change {
                Change::Position(point) => Some((event.at, point)),
                _ => None,
            })
            .collect()
    }

    fn fired(&self) -> bool {
        use std::task::{Context, Poll, Waker};
        let mut cx = Context::from_waker(Waker::noop());
        let fired = std::pin::pin!(self.changed.fired()).poll(&mut cx);
        matches!(fired, Poll::Ready(()))
    }
}

/// Lets the watcher handle everything due now.
async fn settle() {
    for _ in 0..8 {
        yield_now().await;
    }
}

async fn run(test: impl AsyncFnOnce(&LocalSet)) {
    let local = LocalSet::new();
    local.run_until(test(&local)).await;
}

#[tokio::test(start_paused = true)]
async fn asks_follow_motion_once_per_ms_once_at_the_start_and_after_the_last_held_back() {
    run(async |local| {
        let h = Harness::new(local, Corner::default());
        h.moved().await;
        advance(ms(5)).await;
        assert_eq!(h.asker.asked(), [], "asked while idle");

        h.record().await;
        let start = Instant::now();
        assert_eq!(h.asker.asked(), [start], "no ask at the start");
        advance(us(200)).await;
        h.moved().await;
        advance(us(300)).await;
        h.moved().await;
        assert_eq!(h.asker.asked().len(), 1, "asked twice within a ms");
        advance(us(500)).await;
        settle().await;
        assert_eq!(
            h.asker.asked(),
            [start, start + ms(1)],
            "the held-back move"
        );
        advance(ms(3)).await;
        settle().await;
        assert_eq!(h.asker.asked().len(), 2, "asked with no motion");
        h.moved().await;
        assert_eq!(h.asker.asked().len(), 3, "a move after the cap waits");

        h.stop().await;
        h.moved().await;
        advance(ms(20)).await;
        assert_eq!(h.asker.asked().len(), 3, "asked after the stop");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_move_rung_just_before_a_stop_asks_nothing_after_it() {
    run(async |local| {
        let h = Harness::new(local, Corner::default());
        h.record().await;
        advance(ms(5)).await;
        settle().await;
        assert_eq!(h.asker.asked().len(), 1);
        // The watcher hears the stop first, then the move left over.
        h.feed.motion_read(clock(), 1, 0);
        drop(h.slot.take());
        settle().await;
        advance(ms(5)).await;
        settle().await;
        assert_eq!(h.asker.asked().len(), 1, "asked after the stop");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_move_during_an_ask_is_asked_for_once_it_is_answered() {
    run(async |local| {
        let h = Harness::new(local, Corner::default());
        h.asker.then(Reply::After(ms(3), Ok(HERE)));
        h.record().await;
        let start = Instant::now();
        advance(ms(1)).await;
        h.moved().await;
        h.moved().await;
        assert_eq!(h.asker.asked().len(), 1, "two asks in flight");
        advance(ms(2)).await;
        settle().await;
        assert_eq!(h.asker.asked(), [start, start + ms(3)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_answer_is_fed_after_its_hold_moved_by_the_layouts_corner() {
    run(async |local| {
        let h = Harness::new(local, Corner { x: -500, y: -300 });
        h.record().await;
        settle().await;
        advance(HOLD - us(1)).await;
        settle().await;
        advance(us(1)).await;
        settle().await;
        let positions = h.stop().await;
        // Stamped when read, at the start; the first position goes first.
        assert_eq!(positions, [(0, Point { x: 600, y: 500 })]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_held_answer_is_never_fed_before_its_hold_ends() {
    run(async |local| {
        let h = Harness::new(local, Corner::default());
        h.record().await;
        advance(HOLD - us(1)).await;
        settle().await;
        assert_eq!(h.stop().await, [], "fed before its hold ended");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_answer_the_layout_changes_under_is_dropped() {
    run(async |local| {
        // Changed within the hold, even back to the same screens.
        let h = Harness::new(local, Corner::default());
        h.record().await;
        advance(ms(9)).await;
        h.screens.change(Ok(screens(1920)));
        advance(ms(20)).await;
        settle().await;
        assert_eq!(h.stop().await, []);

        // Asked before a change, answered after it.
        let h = Harness::new(local, Corner::default());
        h.asker.then(Reply::After(ms(3), Ok(HERE)));
        h.record().await;
        advance(ms(1)).await;
        h.screens.change(Ok(screens(1920)));
        advance(ms(30)).await;
        settle().await;
        assert_eq!(h.stop().await, []);

        // Held past its hold while a change was heard and not yet settled,
        // as when the daemon was busy: the display reads the change in the
        // same poll in which the hold ends.
        let h = Harness::new(local, Corner::default());
        h.record().await;
        advance(ms(9)).await;
        h.screens.hear();
        advance(ms(20)).await;
        settle().await;
        assert_eq!(h.stop().await, [], "fed while a change was unsettled");

        // Asked while a change was unsettled, answered once it settled
        // back on the same screens.
        let h = Harness::new(local, Corner::default());
        h.screens.hear();
        h.asker.then(Reply::After(ms(3), Ok(HERE)));
        h.record().await;
        advance(ms(1)).await;
        h.screens.change(Ok(screens(1920)));
        advance(ms(30)).await;
        settle().await;
        assert_eq!(h.stop().await, [], "asked while a change was unsettled");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_answer_on_an_unknown_layout_or_outside_its_box_is_dropped() {
    run(async |local| {
        let h = Harness::new(local, Corner::default());
        h.screens.change(Err(NoScreens::Lost));
        h.record().await;
        advance(ms(20)).await;
        settle().await;
        assert_eq!(h.stop().await, []);

        let h = Harness::new(local, Corner { x: 0, y: 300 });
        h.record().await;
        advance(ms(20)).await;
        settle().await;
        assert_eq!(h.stop().await, [], "above the box");

        let h = Harness::new(local, Corner { x: 200, y: 0 });
        h.record().await;
        advance(ms(20)).await;
        settle().await;
        assert_eq!(h.stop().await, [], "left of the box");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_stop_drops_the_ask_in_flight_and_what_is_held_at_once() {
    run(async |local| {
        let h = Harness::new(local, Corner::default());
        h.asker.then(Reply::After(Duration::ZERO, Ok(HERE)));
        h.asker.then(Reply::Never);
        h.record().await;
        advance(ms(2)).await;
        h.moved().await;
        assert_eq!(h.asker.asked().len(), 2);
        let recorder = h.slot.take();
        // Well under the ask's own deadline: the stop ends it, not time.
        timeout(us(10), settle()).await.unwrap();
        assert_eq!(h.asker.dropped(), 1, "the ask in flight was kept");
        drop(recorder);
        advance(ms(100)).await;
        settle().await;
        assert_eq!(h.asker.asked().len(), 2);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_failed_ask_says_why_until_one_is_answered_or_its_recording_stops() {
    run(async |local| {
        let h = Harness::new(local, Corner::default());
        h.asker.then(Reply::After(ms(50), Err(Failure::NoAnswer)));
        h.record().await;
        advance(ms(50)).await;
        settle().await;
        assert_eq!(h.reason.get(), Err(NoPointer::HyprlandNoAnswer));
        assert!(h.fired());
        h.moved().await;
        assert_eq!(h.reason.get(), Ok(()), "an answer did not clear it");
        assert!(h.fired());

        h.asker
            .then(Reply::After(Duration::ZERO, Err(Failure::Unreadable)));
        advance(ms(2)).await;
        h.moved().await;
        assert_eq!(h.reason.get(), Err(NoPointer::HyprlandUnreadable));
        assert!(h.fired());
        h.stop().await;
        assert_eq!(h.reason.get(), Ok(()), "the stop did not clear it");
        assert!(h.fired());
        assert!(!h.fired(), "fired with nothing changed");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_reason_from_before_a_recording_stays_until_an_ask_is_answered() {
    run(async |local| {
        let h = Harness::new(local, Corner::default());
        h.reason.set(Err(NoPointer::HyprlandNoAnswer));
        assert!(h.fired());
        // Started and stopped before its first ask came back.
        h.asker.then(Reply::Never);
        h.record().await;
        h.stop().await;
        assert_eq!(h.reason.get(), Err(NoPointer::HyprlandNoAnswer));
        h.record().await;
        assert_eq!(h.reason.get(), Ok(()));
    })
    .await;
}
