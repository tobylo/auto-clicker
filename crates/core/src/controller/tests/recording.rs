//! Recording through the controller: what starts and stops it, what is
//! saved, and what is refused. The fake reader's feed stands in for the
//! input reader; the recorder's own rules are tested in `recording`.

use std::fs;

use super::*;
use crate::hotkeys::DeviceId;
use crate::recording::{Change, KeyInput, MAX_DURATION, Recording};

pub(super) const KEYBOARD: DeviceId = DeviceId(1);
const MOUSE: DeviceId = DeviceId(2);
const W: u16 = 17;
const F7: u16 = 65;
const BTN_LEFT: u16 = 0x110;
const SAVED: &str = "Saved \u{201c}Recording 2026-11-09 18:04\u{201d}";

impl Daemon {
    /// A key event from the reader, now.
    pub(super) fn key(&self, device: DeviceId, code: u16, pressed: bool) {
        let now = self.shared.clock();
        self.shared
            .feed(|recorder| recorder.key(device, KeyInput::new(code, pressed), now));
    }

    fn motion(&self, dx: i32, dy: i32) {
        let now = self.shared.clock();
        self.shared.feed(|recorder| recorder.motion(now, dx, dy));
    }

    /// Ctrl+F7 as the reader handles it: the recorder sees the press that
    /// fired, then the controller gets the hotkey.
    pub(super) async fn record_hotkey(&self) {
        let now = self.shared.clock();
        self.shared.feed(|recorder| {
            recorder.key(
                KEYBOARD,
                KeyInput::press(F7).firing(Action::ToggleRecording),
                now,
            );
        });
        self.hotkey(HotkeyEvent::Fired(Action::ToggleRecording))
            .await;
    }

    /// The saved recordings, oldest name first.
    pub(super) fn saved(&self) -> Vec<Recording> {
        let dir = self.shared.with(|r| r.recordings.clone()).unwrap();
        dir.entries()
            .iter()
            .map(|name| Recording::from_json(&fs::read(dir.path().join(name)).unwrap()).unwrap())
            .collect()
    }

    /// Asserts the controller is idle again, says `notice`, and saved one
    /// recording of `changes` (times in ms).
    fn assert_saved(&self, notice: &str, changes: &[(u64, Change)]) {
        let shown = (self.activity(), self.notice());
        assert_eq!(shown, (Activity::Idle, notice.to_owned()));
        assert_eq!(self.saved_changes(), changes);
    }

    /// The one saved recording's changes, with their times in ms.
    fn saved_changes(&self) -> Vec<(u64, Change)> {
        let saved = self.saved();
        assert_eq!(saved.len(), 1, "{saved:?}");
        saved[0]
            .events()
            .iter()
            .map(|event| (event.at / 1000, event.change))
            .collect()
    }
}

fn press(code: u16) -> Change {
    Change::Press(crate::recording::recordable(code).unwrap())
}

fn release(code: u16) -> Change {
    Change::Release(crate::recording::recordable(code).unwrap())
}

#[tokio::test(start_paused = true)]
async fn the_hotkey_records_at_once_and_saves_on_the_next_press() {
    local(async {
        let d = daemon(Mode::Click).await;
        d.record_hotkey().await;
        assert_eq!(d.activity(), Activity::Recording);
        sleep(Duration::from_millis(100)).await;
        d.key(KEYBOARD, W, true);
        sleep(Duration::from_millis(400)).await;
        d.key(KEYBOARD, W, false);
        d.motion(3, 4);
        sleep(Duration::from_millis(500)).await;
        d.record_hotkey().await;

        let moved = Change::Move { dx: 3, dy: 4 };
        d.assert_saved(SAVED, &[(100, press(W)), (500, release(W)), (500, moved)]);
        assert_eq!(d.saved()[0].duration_us(), 1_000_000);
        // Recording pressed nothing.
        d.assert_idle_after(&[]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_tray_counts_down_then_records_and_trims_its_own_trip() {
    local(async {
        let d = daemon(Mode::Click).await;
        d.send(Message::Record).await;
        let mut shown = vec![d.activity()];
        for _ in 0..5 {
            sleep(secs(1)).await;
            d.settle().await;
            shown.push(d.activity());
        }
        use Activity::*;
        assert_eq!(
            shown,
            [
                RecordingIn(5),
                RecordingIn(4),
                RecordingIn(3),
                RecordingIn(2),
                RecordingIn(1),
                Recording
            ]
        );
        d.key(KEYBOARD, W, true);
        sleep(secs(1)).await;
        d.key(KEYBOARD, W, false);
        d.key(MOUSE, BTN_LEFT, true);
        sleep(Duration::from_millis(100)).await;
        d.key(MOUSE, BTN_LEFT, false);
        // The trip: to the icon, press there (the menu opens), drag to
        // Stop recording and let go: one click, which only the menu's
        // opening tells apart from the game's.
        sleep(Duration::from_millis(500)).await;
        d.motion(300, -200);
        sleep(Duration::from_millis(300)).await;
        d.key(MOUSE, BTN_LEFT, true);
        d.send(Message::MenuOpened).await;
        sleep(Duration::from_millis(500)).await;
        d.motion(0, -40);
        d.key(MOUSE, BTN_LEFT, false);
        d.send(Message::StopRecording).await;

        assert_eq!(d.notice(), SAVED);
        assert_eq!(
            d.saved_changes(),
            [
                (0, press(W)),
                (1000, release(W)),
                (1000, press(BTN_LEFT)),
                (1100, release(BTN_LEFT))
            ]
        );
        assert_eq!(d.saved()[0].duration(), Duration::from_millis(1600));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_record_countdown_records_nothing() {
    for cancel in ["tray", "hotkey", "lock"] {
        local(async {
            let d = daemon(Mode::Click).await;
            d.send(Message::Record).await;
            sleep(secs(2)).await;
            match cancel {
                "tray" => d.send(Message::StopRecording).await,
                "hotkey" => d.record_hotkey().await,
                _ => d.send(Message::LockRequested).await,
            }
            assert_eq!(d.activity(), Activity::Idle, "{cancel}");
            sleep(secs(10)).await;
            d.settle().await;
            assert_eq!(d.activity(), Activity::Idle, "{cancel}");
            assert!(d.saved().is_empty(), "{cancel}");
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_lock_inactive_session_or_sleep_stops_and_saves_at_once() {
    let stops = [
        (Message::LockRequested, "Stopped: the screen is locked"),
        (
            Message::Session(Some(Blocked::Inactive)),
            "Stopped: another session is in front",
        ),
        (Message::Sleep(1), "Stopped: the computer is going to sleep"),
    ];
    for (message, why) in stops {
        local(async {
            let d = daemon(Mode::Click).await;
            d.record_hotkey().await;
            d.key(KEYBOARD, W, true);
            sleep(secs(2)).await;
            let before = Instant::now();
            d.send(message.clone()).await;
            // At once: before any virtual time passed.
            assert_eq!(Instant::now(), before, "{message:?}");
            d.assert_saved(
                &format!("{why}. {SAVED}"),
                &[(0, press(W)), (2000, release(W))],
            );
            // Saved before sleep was let go.
            if message == Message::Sleep(1) {
                assert_eq!(*d.released_for_sleep.borrow(), 1);
            }
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_recording_stops_and_saves_at_its_time_limit() {
    local(async {
        let d = daemon(Mode::Click).await;
        let start = Instant::now();
        d.record_hotkey().await;
        d.key(KEYBOARD, W, true);
        sleep(MAX_DURATION - secs(1)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Recording);
        sleep(secs(1)).await;
        d.settle().await;
        // Stopped on the deadline itself, not later.
        assert_eq!(Instant::now() - start, MAX_DURATION);
        d.assert_saved(
            &format!("Recording stopped at its 60-minute limit. {SAVED}"),
            &[(0, press(W)), (3_600_000, release(W))],
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn quitting_saves_the_recording() {
    local(async {
        let d = daemon(Mode::Click).await;
        d.record_hotkey().await;
        d.key(KEYBOARD, W, true);
        d.send(Message::Quit).await;
        assert!(d.task.is_finished());
        assert_eq!(d.saved_changes(), [(0, press(W)), (0, release(W))]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_empty_recording_is_not_saved() {
    local(async {
        let d = daemon(Mode::Click).await;
        d.record_hotkey().await;
        sleep(secs(3)).await;
        d.record_hotkey().await;
        assert_eq!(d.notice(), "Nothing was recorded, so nothing was saved");
        assert!(d.saved().is_empty());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_that_cannot_be_saved_says_why() {
    local(async {
        let shared = Shared::default();
        let d = daemon_with(config(Mode::Click), shared.clone(), true).await;
        shared.with(|r| r.recordings = None);
        d.record_hotkey().await;
        d.key(KEYBOARD, W, true);
        d.record_hotkey().await;
        assert_eq!(
            d.notice(),
            "The recording could not be saved: no data directory: neither XDG_DATA_HOME nor \
             HOME is set"
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn recording_is_refused_while_clicking_and_clicking_while_recording() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.start_by_hotkey().await;
        d.record_hotkey().await;
        d.send(Message::Record).await;
        assert_eq!(d.activity(), Activity::Clicking);
        assert_eq!(d.notice(), "Not recording: clicking is on; stop it first");
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::Idle);

        d.record_hotkey().await;
        d.press_hotkey().await;
        d.send(Message::Start).await;
        d.send(Message::Stop).await;
        d.hotkey(HotkeyEvent::Fired(Action::TogglePlayback)).await;
        sleep(secs(10)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Recording);
        assert_eq!(d.shared.calls(), [LEFT_DOWN, LEFT_UP]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_view_says_recording_cannot_start_exactly_when_a_start_is_refused() {
    let no_permission = Unavailable::NoPermission;
    let watch_failed = Unavailable::WatchFailed("gone".to_owned().into());
    let cases = [
        (
            HotkeyStatus::Unavailable(no_permission.clone()),
            Some(NoRecording::Keyboards(no_permission)),
        ),
        (HotkeyStatus::Available, None),
        // Recording reads the mice and later keyboards still.
        (HotkeyStatus::Unavailable(Unavailable::NoKeyboard), None),
        (
            HotkeyStatus::Unavailable(Unavailable::ReadFailed("busy".to_owned().into())),
            None,
        ),
        (
            HotkeyStatus::Unavailable(Unavailable::Desktop("gone".to_owned().into())),
            None,
        ),
        (
            HotkeyStatus::Unavailable(watch_failed.clone()),
            Some(NoRecording::Keyboards(watch_failed)),
        ),
    ];
    local(async {
        let d = daemon(Mode::Click).await;
        assert_eq!(d.view().no_recording, None);
        for (status, no_recording) in cases {
            d.hotkey(HotkeyEvent::Status(status.clone())).await;
            assert_eq!(d.view().no_recording, no_recording, "{status:?}");
            d.send(Message::Record).await;
            match &no_recording {
                Some(why) => {
                    assert_eq!(d.activity(), Activity::Idle, "{status:?}");
                    assert_eq!(d.notice(), format!("Not recording: {why}"));
                }
                None => {
                    assert!(d.activity().recording(), "{status:?}");
                    d.send(Message::StopRecording).await;
                }
            }
        }
        assert!(d.saved().is_empty());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn recording_is_refused_when_the_session_devices_or_clock_do_not_allow_it() {
    type SetUp = Box<dyn Fn(&Shared)>;
    let cases: [(&str, SetUp, bool); 3] = [
        (
            "Not recording: the screen is locked",
            Box::new(|_| {}),
            false,
        ),
        (
            "Not recording: the clock could not be read",
            Box::new(|shared| shared.with(|r| r.no_local_time = true)),
            true,
        ),
        (
            "Not recording: hotkeys are unavailable: keyboards cannot be read; add your user to \
             the input group and log in again, or set hotkeys = \"portal\" under [input] for the \
             desktop's shortcuts",
            Box::new(|shared| {
                shared.hotkey(HotkeyEvent::Status(HotkeyStatus::Unavailable(
                    Unavailable::NoPermission,
                )));
            }),
            true,
        ),
    ];
    for (notice, set_up, session_open) in cases {
        local(async {
            let config = Config {
                tray_start_delay: Duration::ZERO,
                ..config(Mode::Click)
            };
            let d = daemon_with(config, Shared::default(), session_open).await;
            if !session_open {
                d.send(Message::Session(Some(Blocked::Locked))).await;
            }
            set_up(&d.shared);
            d.settle().await;
            for start in ["hotkey", "tray"] {
                match start {
                    "hotkey" => d.record_hotkey().await,
                    _ => d.send(Message::Record).await,
                }
                assert_eq!(d.activity(), Activity::Idle, "{start}");
                assert_eq!(d.notice(), notice, "{start}");
            }
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn nothing_the_tray_shows_names_a_recorded_key() {
    local(async {
        let d = daemon(Mode::Click).await;
        let mut shown = Vec::new();
        let mut look = |d: &Daemon| {
            let view = d.view();
            shown.push(view.title());
            shown.extend(view.details());
        };
        d.record_hotkey().await;
        look(&d);
        for code in [W, 30, 31, 32] {
            d.key(KEYBOARD, code, true);
            d.key(KEYBOARD, code, false);
        }
        d.settle().await;
        look(&d);
        d.record_hotkey().await;
        look(&d);
        let text = shown.join("\n");
        assert!(text.contains(SAVED), "{text}");
        for name in ["KEY_", "17", "30", "Key("] {
            assert!(!text.contains(name), "{name} in {text}");
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn losing_the_input_reader_stops_and_saves_or_cancels_at_once() {
    let lost = || {
        HotkeyEvent::Status(HotkeyStatus::Unavailable(Unavailable::WatchFailed(
            "gone".to_owned().into(),
        )))
    };
    let why = "hotkeys stopped: watching for keyboards failed: gone; restart the program";
    local(async {
        let d = daemon(Mode::Click).await;
        d.record_hotkey().await;
        d.key(KEYBOARD, W, true);
        sleep(secs(1)).await;
        let before = Instant::now();
        d.hotkey(lost()).await;
        assert_eq!(Instant::now(), before);
        d.assert_saved(
            &format!("Recording stopped: {why}. {SAVED}"),
            &[(0, press(W)), (1000, release(W))],
        );
    })
    .await;
    local(async {
        let d = daemon(Mode::Click).await;
        d.hotkey(lost()).await;
        for start in [Message::Record, Message::Record] {
            d.send(start).await;
            assert_eq!(d.notice(), format!("Not recording: {why}"));
        }
        d.record_hotkey().await;
        assert_eq!(d.activity(), Activity::Idle);
    })
    .await;
    local(async {
        let d = daemon(Mode::Click).await;
        d.send(Message::Record).await;
        d.hotkey(lost()).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(d.notice(), format!("Not recording: {why}"));
        sleep(secs(10)).await;
        d.settle().await;
        assert!(d.saved().is_empty());
    })
    .await;
}
