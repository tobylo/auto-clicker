//! The cursor sessions against a fake display at a path of its own: when a
//! connection is made and closed, what reaches the recorder, and what the
//! reason says.

use std::cell::Cell;
use std::num::NonZeroU32;
use std::time::Instant as StdInstant;

use auto_clicker_core::hotkeys::Bindings;
use auto_clicker_core::recording::{
    Change, Created, Ending, Layout, Point, Recorder, RecordingSlot,
};
use tokio::task::{JoinHandle, LocalSet, yield_now};
use tokio::time::timeout;

use super::*;
use crate::client::Link;
use crate::client::fake::{
    A, B, BOUND, C, CAPTURE, Display, Fake, LEFT, Message, Offer, RIGHT, SEAT, SOURCES, layout,
};

thread_local! {
    static EPOCH: Cell<Option<StdInstant>> = const { Cell::new(None) };
}

/// The event clock, from one second.
fn clock() -> Duration {
    let epoch = EPOCH.get().unwrap_or_else(|| {
        let now = StdInstant::now();
        EPOCH.set(Some(now));
        now
    });
    Duration::from_secs(1) + epoch.elapsed()
}

/// Well under [`SETUP`]: a stop that waits it out fails.
const AT_ONCE: Duration = Duration::from_millis(300);

fn two_screens() -> Layout {
    layout(&[LEFT, RIGHT]).unwrap()
}

/// A display at its own path that offered cursor sessions (or not) at
/// start, the cursor sessions' watcher on it, and a recording slot.
struct Harness {
    display: Display,
    cursors: Cursors,
    slot: RecordingSlot,
    open: Rc<Cell<bool>>,
    fired: Rc<Cell<usize>>,
    /// The layout connection's far end, kept open.
    layout: Fake,
    _watch: JoinHandle<()>,
}

impl Harness {
    async fn new(offered: bool) -> Harness {
        Harness::listened_by(offered, NonZeroU32::new(std::process::id())).await
    }

    /// The same, the display's listener found at start being `compositor`.
    async fn listened_by(offered: bool, compositor: Option<NonZeroU32>) -> Harness {
        let display = Display::new();
        let (link, mut fake) = Fake::pair();
        fake.cursors = offered;
        let (started, ()) = timeout(BOUND, async {
            tokio::join!(Link::start(link), fake.start_with_two())
        })
        .await
        .unwrap();
        let (link, first) = started.unwrap();
        let wayland = Wayland::of(compositor, Some(display.path.clone()), first, Ok(link));
        let cursors = Cursors::of(&wayland, clock);
        let slot = RecordingSlot::new();
        let open = Rc::new(Cell::new(true));
        let opens = Rc::clone(&open);
        let changed = LayoutSignal::default();
        let fired = Rc::new(Cell::new(0));
        let seen = Rc::clone(&fired);
        let signal = changed.clone();
        tokio::task::spawn_local(async move {
            loop {
                signal.fired().await;
                seen.set(seen.get() + 1);
            }
        });
        let watch = cursors.watch(slot.feed(), Box::new(move || opens.get()), changed);
        Harness {
            display,
            cursors,
            slot,
            open,
            fired,
            layout: fake,
            _watch: tokio::task::spawn_local(watch),
        }
    }

    async fn record(&self) {
        let created = Created::parse("2026-11-16T12:00:00.000+01:00").unwrap();
        let recorder = Recorder::new(Bindings::default(), clock(), created, None);
        self.slot.start(recorder.with_layout(two_screens()));
        settle().await;
    }

    /// The recording's connection, accepted and set up: two outputs side
    /// by side, a seat with a pointer, a cursor session on each.
    async fn connected(&self) -> Fake {
        let mut fake = timeout(BOUND, self.display.accept()).await.unwrap();
        fake.cursors = true;
        timeout(BOUND, async {
            fake.start_with_two().await;
            fake.seat(true).await;
            fake.until_sessions(2).await;
        })
        .await
        .unwrap();
        fake
    }

    /// Stops the recording, with a wheel turn so it is saved; its
    /// positions.
    async fn stop(&self) -> Vec<Point> {
        let mut recorder = self.slot.take().unwrap();
        settle().await;
        recorder.wheel(clock(), 1);
        let recording = recorder.finish(Ending::At(clock())).unwrap().unwrap();
        if recording.layout().is_some() {
            assert_eq!(
                recording.positions_from(),
                Some(PointerSource::CursorSessions)
            );
        }
        recording
            .events()
            .iter()
            .filter_map(|event| match event.change {
                Change::Position(point) => Some(point),
                _ => None,
            })
            .collect()
    }

    /// Whether a connection was made to the display, waiting `wait`.
    async fn connects_within(&self, wait: Duration) -> bool {
        timeout(wait, self.display.accept()).await.is_ok()
    }
}

/// Lets every ready task run.
async fn settle() {
    for _ in 0..16 {
        yield_now().await;
    }
}

/// Sends `messages`, then makes sure the client read them: an output's
/// `done` after them has it ask for a round trip, which is answered. The
/// client fed what it read before it sent the request.
async fn sent(fake: &mut Fake, mut messages: Vec<Message>) {
    let output = fake.ids(A).output;
    messages.push(Message {
        object: output,
        opcode: 2,
        words: Vec::new(),
    });
    timeout(BOUND, async {
        fake.send(messages).await;
        fake.round_trip().await;
    })
    .await
    .unwrap();
    settle().await;
}

fn at(x: u32, y: u32) -> Point {
    Point { x, y }
}

async fn local(test: impl Future<Output = ()>) {
    LocalSet::new().run_until(test).await;
}

#[tokio::test]
async fn a_recording_gets_each_position_on_its_output_the_first_without_a_move() {
    local(async {
        let h = Harness::new(true).await;
        h.record().await;
        let mut fake = h.connected().await;
        // The position niri sends as each session is made, then a move
        // onto the right-hand output; the left one's session reports the
        // cursor's image still overlapping it, off the output.
        let positions = vec![
            fake.position(A, 100, 200),
            fake.position(B, 10, 20),
            fake.position(A, 2050, 20),
        ];
        sent(&mut fake, positions).await;
        assert_eq!(h.cursors.found(), Some(Ok(())));
        assert_eq!(h.stop().await, [at(100, 200), at(2058, 20)]);
    })
    .await;
}

#[tokio::test]
async fn nothing_is_asked_of_the_display_while_no_recording_runs() {
    local(async {
        let h = Harness::new(true).await;
        settle().await;
        assert!(!h.connects_within(AT_ONCE).await);
        assert_eq!(h.cursors.found(), Some(Ok(())));
        // The layout connection only noted them.
        let bound: Vec<_> = h.layout.bound.iter().map(|(name, _)| *name).collect();
        for name in [SEAT, CAPTURE, SOURCES] {
            assert!(!bound.contains(&name), "{name:?} bound");
        }
    })
    .await;
}

#[tokio::test]
async fn a_recording_opens_nothing_where_kwin_or_hyprland_reports() {
    local(async {
        let h = Harness::new(true).await;
        h.open.set(false);
        h.record().await;
        assert!(!h.connects_within(AT_ONCE).await);
    })
    .await;
}

#[tokio::test]
async fn a_display_that_did_not_offer_them_has_none_and_watches_nothing() {
    local(async {
        let h = Harness::new(false).await;
        assert_eq!(h.cursors.found(), None);
        assert!(!h.cursors.0.watched.get());
        h.record().await;
        assert!(!h.connects_within(AT_ONCE).await);
    })
    .await;
}

#[tokio::test]
async fn only_the_first_watch_watches() {
    local(async {
        let h = Harness::new(true).await;
        assert!(h.cursors.0.watched.get());
        let slot = RecordingSlot::new();
        let second = h
            .cursors
            .watch(slot.feed(), Box::new(|| true), LayoutSignal::default());
        slot.start(Recorder::new(
            Bindings::default(),
            clock(),
            Created::parse("2026-11-16T12:00:00.000+01:00").unwrap(),
            None,
        ));
        tokio::task::spawn_local(second);
        assert!(!h.connects_within(AT_ONCE).await);
    })
    .await;
}

#[tokio::test]
async fn a_stop_closes_the_connection_at_once_while_it_is_set_up() {
    local(async {
        let h = Harness::new(true).await;
        h.record().await;
        let mut fake = timeout(BOUND, h.display.accept()).await.unwrap();
        // Nothing answered: the setup waits.
        h.stop().await;
        assert!(timeout(AT_ONCE, fake.closed()).await.unwrap());
    })
    .await;
}

#[tokio::test]
async fn a_stop_closes_the_connection_at_once_while_it_is_read() {
    local(async {
        let h = Harness::new(true).await;
        h.record().await;
        let mut fake = h.connected().await;
        h.stop().await;
        assert!(timeout(AT_ONCE, fake.closed()).await.unwrap());
        assert_eq!(h.cursors.found(), Some(Ok(())));
    })
    .await;
}

#[tokio::test]
async fn a_display_that_sends_no_position_gives_none_and_no_reason() {
    local(async {
        let h = Harness::new(true).await;
        h.record().await;
        let mut fake = h.connected().await;
        sent(&mut fake, Vec::new()).await;
        assert_eq!(h.cursors.found(), Some(Ok(())));
        assert_eq!(h.fired.get(), 0);
        assert_eq!(h.stop().await, []);
        assert!(timeout(AT_ONCE, fake.closed()).await.unwrap());
    })
    .await;
}

#[tokio::test]
async fn a_stop_and_a_start_told_at_once_are_a_new_connection() {
    local(async {
        let h = Harness::new(true).await;
        h.record().await;
        let mut first = h.connected().await;
        h.slot.take().unwrap();
        h.record().await;
        assert!(timeout(AT_ONCE, first.closed()).await.unwrap());
        let mut second = h.connected().await;
        let position = second.position(A, 1, 2);
        sent(&mut second, vec![position]).await;
        assert_eq!(h.stop().await, [at(1, 2)]);
    })
    .await;
}

#[tokio::test]
async fn a_position_heard_before_an_update_settles_is_dropped() {
    local(async {
        let h = Harness::new(true).await;
        h.record().await;
        let mut fake = h.connected().await;
        let xdg = fake.ids(A).xdg;
        // The same place told again, then a position before its `done`:
        // the update could have moved the outputs.
        let update = Message {
            object: xdg,
            opcode: 0,
            words: vec![0, 0],
        };
        let dropped = fake.position(A, 5, 5);
        sent(&mut fake, vec![update, dropped]).await;
        let kept = fake.position(A, 7, 7);
        sent(&mut fake, vec![kept]).await;
        assert_eq!(h.stop().await, [at(7, 7)]);
    })
    .await;
}

#[tokio::test]
async fn an_output_plugged_in_during_a_recording_gets_a_session() {
    local(async {
        let h = Harness::new(true).await;
        h.record().await;
        let mut fake = h.connected().await;
        timeout(BOUND, async {
            fake.offer(Offer::Output(C)).await;
            fake.until_sessions(3).await;
            // The round trip its coming asked for.
            fake.round_trip().await;
        })
        .await
        .unwrap();
        // Unplugged again: its session and source are destroyed.
        let session = fake.session(C);
        let (source, _) = *fake.sources.last().unwrap();
        let removed = Message {
            object: crate::client::fake::REGISTRY,
            opcode: 1,
            words: vec![C.0],
        };
        sent(&mut fake, vec![removed]).await;
        assert!(fake.seen.contains(&(session, 0)), "session not destroyed");
        assert!(fake.seen.contains(&(source, 0)), "source not destroyed");
        h.stop().await;
        assert!(timeout(AT_ONCE, fake.closed()).await.unwrap());
    })
    .await;
}

#[tokio::test]
async fn a_seat_without_a_pointer_says_why_until_the_recording_stops() {
    local(async {
        let h = Harness::new(true).await;
        h.record().await;
        let mut fake = timeout(BOUND, h.display.accept()).await.unwrap();
        fake.cursors = true;
        timeout(BOUND, async {
            fake.start_with_two().await;
            fake.seat(false).await;
        })
        .await
        .unwrap();
        assert!(timeout(BOUND, fake.closed()).await.unwrap());
        settle().await;
        assert_eq!(h.cursors.found(), Some(Err(NoPointer::DisplayNoPosition)));
        assert_eq!(h.fired.get(), 1);
        h.stop().await;
        assert_eq!(h.cursors.found(), Some(Ok(())));
        assert_eq!(h.fired.get(), 2);
    })
    .await;
}

#[tokio::test]
async fn a_connection_lost_during_the_recording_says_why() {
    local(async {
        let h = Harness::new(true).await;
        h.record().await;
        let fake = h.connected().await;
        drop(fake);
        timeout(BOUND, async {
            while h.cursors.found() == Some(Ok(())) {
                settle().await;
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(h.cursors.found(), Some(Err(NoPointer::DisplayNoPosition)));
        h.stop().await;
        assert_eq!(h.cursors.found(), Some(Ok(())));
    })
    .await;
}

#[tokio::test]
async fn a_setup_past_its_deadline_says_why_and_closes() {
    local(async {
        let h = Harness::new(true).await;
        h.record().await;
        let mut fake = timeout(BOUND, h.display.accept()).await.unwrap();
        let started = StdInstant::now();
        assert!(timeout(BOUND, fake.closed()).await.unwrap());
        assert!(started.elapsed() >= SETUP - Duration::from_millis(50));
        settle().await;
        assert_eq!(h.cursors.found(), Some(Err(NoPointer::DisplayNoPosition)));
    })
    .await;
}

#[tokio::test]
async fn a_display_another_process_listens_on_now_gives_no_position() {
    local(async {
        let other = NonZeroU32::new(u32::MAX - 1);
        let h = Harness::listened_by(true, other).await;
        h.record().await;
        // The connection is made, its listener checked and dropped.
        let mut fake = timeout(BOUND, h.display.accept()).await.unwrap();
        assert!(timeout(AT_ONCE, fake.closed()).await.unwrap());
        settle().await;
        assert_eq!(h.cursors.found(), Some(Err(NoPointer::DisplayNoPosition)));
    })
    .await;
}
