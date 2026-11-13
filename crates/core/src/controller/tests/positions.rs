//! Positions through the controller: the layout a recording gets, what the
//! tray says when positions cannot play, and the stop on a layout change.

use std::fs;

use super::*;
use crate::input::AbsolutePosition;
use crate::playback::Fallback;
use crate::recording::tests::positions::screen;
use crate::recording::tests::positions::{at, two_screens};
use crate::recording::{
    Change, Created, Event, FileName, Layout, Point, Recording, recordable, save,
};
use crate::recording::{PointerSource, Track};

const W: u16 = 17;
const HELD_FOR: Duration = Duration::from_millis(500);

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn w(state: PressState) -> Call {
    Call::Key(Key::new(W).unwrap(), state)
}

/// The two screens with the panel at scale 1.5.
fn rescaled() -> Layout {
    Layout::new(vec![
        screen([0, 0, 2048, 1280], 1500),
        screen([2048, 0, 3440, 1440], 1000),
    ])
    .unwrap()
}

fn panel_only() -> Layout {
    Layout::new(vec![screen([0, 0, 2048, 1280], 1250)]).unwrap()
}

/// W held from 10 ms to 500 ms in a recording of a second, with the
/// pointer at (2744, 720) on DP-1 at the start; with no positions, a
/// version 1 recording of the same keys.
fn store(shared: &Shared, with_positions: bool) -> FileName {
    store_with(shared, with_positions, &[])
}

/// The same, with `extra` positions too.
fn store_with(shared: &Shared, with_positions: bool, extra: &[Event]) -> FileName {
    let held = recordable(W).unwrap();
    let mut events = vec![
        Event {
            at: 10_000,
            change: Change::Press(held),
        },
        Event {
            at: 500_000,
            change: Change::Release(held),
        },
    ];
    let created = Created::parse("2026-11-09T18:00:00.000+01:00").unwrap();
    let recording = if with_positions {
        events.insert(
            0,
            Event {
                at: 0,
                change: at(2_744, 720),
            },
        );
        events.extend_from_slice(extra);
        events.sort_by_key(|event| event.at);
        Recording::with_positions(
            "stored".into(),
            created,
            1_000_000,
            Track {
                layout: two_screens(),
                source: PointerSource::Kwin,
                events,
            },
        )
    } else {
        Recording::new("stored".into(), created, 1_000_000, events)
    }
    .unwrap();
    let dir = shared.with(|r| r.recordings.clone()).unwrap();
    let path = save(dir.path(), &recording).unwrap();
    FileName::parse(path.file_name().unwrap().to_str().unwrap()).unwrap()
}

/// A daemon that plays at once from the tray, its platform reporting
/// `layout`, with one recording stored.
async fn daemon_playing(layout: Option<Layout>, with_positions: bool) -> (Daemon, FileName) {
    let shared = Shared::default();
    shared.with(|r| {
        r.recordings = Some(Arc::new(TempDir::new()));
        r.layout = layout;
    });
    let file = store(&shared, with_positions);
    let config = Config {
        tray_start_delay: Duration::ZERO,
        ..Config::default()
    };
    (daemon_with(config, shared, true).await, file)
}

impl Daemon {
    /// Plays `file` from the tray until W is down.
    async fn play_file_until_w_is_down(&self, file: &FileName) {
        self.send(Message::Play(file.clone())).await;
        sleep(ms(20)).await;
        self.settle().await;
        assert_eq!(self.activity(), Activity::Playing);
        assert!(self.shared.calls().contains(&w(PressState::Down)));
    }

    pub(super) fn set_layout(&self, layout: Option<Layout>) {
        self.shared.with(|r| r.layout = layout);
    }
}

#[tokio::test(start_paused = true)]
async fn a_version_1_recording_plays_as_before_with_nothing_new_shown() {
    local(async {
        for layout in [None, Some(two_screens())] {
            let (d, file) = daemon_playing(layout, false).await;
            d.play_file_until_w_is_down(&file).await;
            assert_eq!(d.view().notice, None);
            sleep(ms(1000)).await;
            d.settle().await;
            assert_eq!(d.shared.calls(), [w(PressState::Down), w(PressState::Up)]);
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn positions_play_as_absolute_moves_on_the_recordings_layout() {
    local(async {
        let (d, file) = daemon_playing(Some(two_screens()), true).await;
        d.play_file_until_w_is_down(&file).await;
        assert_eq!(d.view().notice, None);
        assert_eq!(
            d.shared.calls(),
            [
                Call::MoveAbsolute(AbsolutePosition { x: 32768, y: 32768 }),
                w(PressState::Down)
            ]
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn positions_that_cannot_play_fall_back_and_say_why() {
    local(async {
        for (layout, why) in [
            (None, Fallback::Unknown),
            (Some(panel_only()), Fallback::Changed),
        ] {
            let (d, file) = daemon_playing(layout, true).await;
            d.play_file_until_w_is_down(&file).await;
            assert!(
                why.to_string()
                    .starts_with("Playing with relative motion: ")
            );
            assert_eq!(d.notice(), why.to_string());
            assert_eq!(d.shared.calls(), [w(PressState::Down)]);
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_layout_is_compared_when_the_run_begins_not_when_it_is_chosen() {
    local(async {
        let (d, file) = daemon_playing(None, true).await;
        d.set_layout(Some(two_screens()));
        d.play_file_until_w_is_down(&file).await;
        assert_eq!(d.view().notice, None);
        assert!(matches!(d.shared.calls()[0], Call::MoveAbsolute(_)));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_layout_change_stops_a_playback_using_positions_at_once() {
    local(async {
        for (now, why) in [
            (
                Some(panel_only()),
                "Playback stopped: the screen layout changed",
            ),
            (
                None,
                "Playback stopped: the screen layout is no longer known",
            ),
        ] {
            let (d, file) = daemon_playing(Some(two_screens()), true).await;
            d.play_file_until_w_is_down(&file).await;
            let asked = Instant::now();
            d.set_layout(now);
            d.send(Message::LayoutChanged).await;
            d.settle().await;
            // Stopped within a few polls, with the paused clock not moved:
            // well before W would come up by itself, at 500 ms.
            assert!(asked.elapsed() < HELD_FOR / 10, "{:?}", asked.elapsed());
            assert_eq!((d.activity(), d.notice()), (Activity::Idle, why.to_owned()));
            assert_eq!(d.shared.calls().last(), Some(&w(PressState::Up)));
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_layout_watchers_signal_stops_a_playback_using_positions_at_once() {
    local(async {
        let (d, file) = daemon_playing(Some(two_screens()), true).await;
        let watches = d.shared.with(|r| r.watches.len());
        assert_eq!(watches, 1, "one watcher, started once");
        d.play_file_until_w_is_down(&file).await;
        let asked = Instant::now();
        d.set_layout(Some(panel_only()));
        d.shared.with(|r| r.watches[0].1.changed());
        d.settle().await;
        assert!(asked.elapsed() < HELD_FOR / 10, "{:?}", asked.elapsed());
        let why = "Playback stopped: the screen layout changed";
        assert_eq!((d.activity(), d.notice()), (Activity::Idle, why.to_owned()));
        assert_eq!(d.shared.calls().last(), Some(&w(PressState::Up)));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_layout_change_is_handled_before_a_playback_event_due_with_it() {
    local(async {
        let shared = Shared::default();
        shared.with(|r| {
            r.recordings = Some(Arc::new(TempDir::new()));
            r.layout = Some(two_screens());
        });
        let later = Event {
            at: 300_000,
            change: at(3_000, 720),
        };
        let file = store_with(&shared, true, &[later]);
        let config = Config {
            tray_start_delay: Duration::ZERO,
            ..Config::default()
        };
        let d = daemon_with(config, shared, true).await;
        d.play_file_until_w_is_down(&file).await;
        // The watcher fires its signal at 250 ms, and the position at
        // 300 ms is due on the same turn of the clock: the change wins.
        d.set_layout(Some(panel_only()));
        let signal = d.shared.with(|r| r.watches[0].1.clone());
        // W went down 20 ms into the run.
        let at_250 = Instant::now() + ms(230);
        tokio::task::spawn_local(async move {
            tokio::time::sleep_until(at_250).await;
            signal.changed();
        });
        d.settle().await;
        tokio::time::advance(ms(400)).await;
        d.settle().await;
        let why = "Playback stopped: the screen layout changed";
        assert_eq!((d.activity(), d.notice()), (Activity::Idle, why.to_owned()));
        let moves = d.shared.calls();
        let moves = moves
            .iter()
            .filter(|call| matches!(call, Call::MoveAbsolute(_)));
        assert_eq!(moves.count(), 1, "the later position played");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_layout_the_watcher_has_queued_is_taken_before_a_playback_event_due_with_it() {
    local(async {
        let shared = Shared::default();
        shared.with(|r| {
            r.recordings = Some(Arc::new(TempDir::new()));
            r.layout = Some(two_screens());
        });
        let later = Event {
            at: 300_000,
            change: at(3_000, 720),
        };
        let file = store_with(&shared, true, &[later]);
        let config = Config {
            tray_start_delay: Duration::ZERO,
            ..Config::default()
        };
        let d = daemon_with(config, shared, true).await;
        d.play_file_until_w_is_down(&file).await;
        // A report of the change waits in the watcher's queue, unread, when
        // the position at 300 ms falls due: the change wins.
        d.shared.with(|r| r.queued_layout = Some(panel_only()));
        tokio::time::advance(ms(400)).await;
        d.settle().await;
        let why = "Playback stopped: the screen layout changed";
        assert_eq!((d.activity(), d.notice()), (Activity::Idle, why.to_owned()));
        let moves = d.shared.calls();
        let moves = moves
            .iter()
            .filter(|call| matches!(call, Call::MoveAbsolute(_)));
        assert_eq!(moves.count(), 1, "the later position played");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_layout_watcher_feeds_the_recording_the_controller_starts() {
    local(async {
        let shared = Shared::default();
        shared.with(|r| r.layout = Some(two_screens()));
        let config = Config {
            tray_start_delay: Duration::ZERO,
            ..Config::default()
        };
        let d = daemon_with(config, shared, true).await;
        assert!(
            d.shared.with(|r| r.watch_polls) > 0,
            "the watcher is driven"
        );
        let feed = d.shared.with(|r| r.watches[0].0.clone());
        assert!(!feed.recording());
        d.send(Message::Record).await;
        d.settle().await;
        assert!(feed.recording());
        d.send(Message::StopRecording).await;
        d.settle().await;
        assert!(!feed.recording());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_relative_playback_or_an_unchanged_layout_carries_on() {
    local(async {
        let cases = [
            // Relative only: no positions, or positions that fell back.
            (Some(two_screens()), false, None),
            (None, true, Some(two_screens())),
            // Told of a change that left the layout as it was, or changed
            // only a scale.
            (Some(two_screens()), true, Some(two_screens())),
            (Some(two_screens()), true, Some(rescaled())),
        ];
        for (layout, with_positions, now) in cases {
            let (d, file) = daemon_playing(layout, with_positions).await;
            d.play_file_until_w_is_down(&file).await;
            d.set_layout(now);
            d.send(Message::LayoutChanged).await;
            d.settle().await;
            assert_eq!(d.activity(), Activity::Playing);
            sleep(ms(1000)).await;
            d.settle().await;
            assert_eq!(d.shared.calls().last(), Some(&w(PressState::Up)));
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_gets_the_layout_current_at_its_start() {
    local(async {
        let shared = Shared::default();
        shared.with(|r| r.layout = Some(two_screens()));
        let d = daemon_with(config(Mode::Click), shared, true).await;
        d.record_hotkey().await;
        sleep(ms(10)).await;
        let now = d.shared.clock();
        let layout = two_screens();
        d.shared.feed(|recorder| {
            recorder.position(
                PointerSource::Kwin,
                now,
                Point { x: 5, y: 6 },
                Some(&layout),
            )
        });
        sleep(ms(10)).await;
        d.key(super::recording::KEYBOARD, W, true);
        d.key(super::recording::KEYBOARD, W, false);
        d.record_hotkey().await;
        let dir = d.shared.with(|r| r.recordings.clone()).unwrap();
        let saved = Recording::from_json(&fs::read(dir.path().join(&dir.entries()[0])).unwrap());
        let saved = saved.unwrap();
        assert_eq!(saved.layout(), Some(&two_screens()));
        assert_eq!(saved.events()[0].change, at(5, 6));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_that_fills_up_stops_and_saves_at_once() {
    local(async {
        let d = daemon(Mode::Click).await;
        d.record_hotkey().await;
        d.shared.feed(|recorder| {
            let small = std::mem::replace(recorder, recorder_stub()).with_budget(200);
            *recorder = small;
        });
        for _ in 0..20 {
            sleep(ms(10)).await;
            d.key(super::recording::KEYBOARD, W, true);
            d.key(super::recording::KEYBOARD, W, false);
        }
        d.settle().await;
        assert_eq!(d.activity(), Activity::Idle);
        assert!(
            d.notice().starts_with(
                "Recording stopped: it reached the largest size a recording may have. Saved"
            ),
            "{}",
            d.notice()
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_wake_left_from_a_full_recording_does_not_stop_the_next() {
    local(async {
        let d = daemon(Mode::Click).await;
        d.record_hotkey().await;
        d.shared.feed(|recorder| {
            let full = std::mem::replace(recorder, recorder_stub()).with_budget(0);
            *recorder = full;
            // The second turn writes the first, which does not fit.
            recorder.wheel(ms(1), 1);
            recorder.wheel(ms(5), 1);
            assert!(recorder.is_full());
        });
        // Stopped and started again before the controller sees the wake.
        d.shared.hotkey(HotkeyEvent::Fired(Action::ToggleRecording));
        d.shared.hotkey(HotkeyEvent::Fired(Action::ToggleRecording));
        d.settle().await;
        assert_eq!(d.activity(), Activity::Recording);
    })
    .await;
}

fn recorder_stub() -> Recorder {
    Recorder::new(
        crate::hotkeys::Bindings::default(),
        Duration::ZERO,
        crate::recording::tests::created(),
        None,
    )
}
