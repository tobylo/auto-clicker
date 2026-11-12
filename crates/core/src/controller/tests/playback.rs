//! Playback through the controller: what starts and stops it, which
//! recording plays, and what is refused. The playback run's own schedule is
//! tested in `playback`.

use std::fs;
use std::num::NonZeroU32;

use super::per_run::per_run_daemon;
use super::recording::KEYBOARD;
use super::*;
use crate::playback::{PlaybackSettings, Speed};
use crate::recording::{Change, Created, Event, FileName, Recording, recordable, save};

const W: u16 = 17;
const LABEL: &str = "Recording 2026-11-09 18:00:00";

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn w(state: PressState) -> Call {
    Call::Key(Key::new(W).unwrap(), state)
}

const PLAY: HotkeyEvent = HotkeyEvent::Fired(Action::TogglePlayback);

/// A config that plays `repeat`, with no tray countdown.
fn playing(repeat: Repeat) -> Config {
    Config {
        tray_start_delay: Duration::ZERO,
        playback: PlaybackSettings {
            repeat,
            ..PlaybackSettings::default()
        },
        ..Config::default()
    }
}

fn once() -> Repeat {
    Repeat::Times(NonZeroU32::MIN)
}

/// A recordings folder to fill before the daemon starts.
fn shared_with_recordings() -> Shared {
    let shared = Shared::default();
    shared.with(|r| r.recordings = Some(Arc::new(TempDir::new())));
    shared
}

impl Shared {
    fn recordings_path(&self) -> PathBuf {
        self.with(|r| r.recordings.as_ref().unwrap().path().to_path_buf())
    }

    /// Saves a recording made at `time` ("18:00:00") on 2026-11-09, as long
    /// as `duration_ms`, of `events` (times in ms); its file.
    pub(super) fn store(&self, time: &str, duration_ms: u64, events: &[(u64, Change)]) -> FileName {
        let created = Created::parse(&format!("2026-11-09T{time}.000+01:00")).unwrap();
        let events = events
            .iter()
            .map(|&(at, change)| Event {
                at: at * 1000,
                change,
            })
            .collect();
        let recording =
            Recording::new(created.default_name(), created, duration_ms * 1000, events).unwrap();
        let path = save(&self.recordings_path(), &recording).unwrap();
        FileName::parse(path.file_name().unwrap().to_str().unwrap()).unwrap()
    }

    /// W held from 10 ms to 500 ms, in a recording of a second, made at
    /// 18:00:00.
    pub(super) fn store_hold(&self) -> FileName {
        let held = recordable(W).unwrap();
        let events = [(10, Change::Press(held)), (500, Change::Release(held))];
        self.store("18:00:00", 1000, &events)
    }

    /// A recording that turns the wheel by `notches` at once.
    fn store_wheel(&self, time: &str, notches: i32) -> FileName {
        self.store(time, 100, &[(0, Change::Wheel(notches))])
    }
}

/// A daemon with the hold recording stored, and that recording's file.
async fn daemon_with_hold(config: Config) -> (Daemon, FileName) {
    let shared = shared_with_recordings();
    let file = shared.store_hold();
    (daemon_with(config, shared, true).await, file)
}

impl Daemon {
    /// Starts playing the hold recording from the hotkey and waits until W
    /// is down.
    async fn play_until_w_is_down(&self) {
        self.hotkey(PLAY).await;
        self.hotkey(HotkeyEvent::ModifiersReleased).await;
        sleep(ms(20)).await;
        self.settle().await;
        assert_eq!(self.activity(), Activity::Playing);
        assert_eq!(self.shared.calls(), [w(PressState::Down)]);
    }

    fn listed(&self) -> Vec<String> {
        let view = self.view();
        view.recordings
            .iter()
            .map(|listed| listed.label.clone())
            .collect()
    }
}

// Starting

#[tokio::test(start_paused = true)]
async fn the_tray_counts_down_plays_and_says_when_it_finished() {
    local(async {
        let config = Config {
            tray_start_delay: secs(5),
            ..playing(once())
        };
        let (d, file) = daemon_with_hold(config).await;
        let start = Instant::now();

        d.send(Message::Play(file)).await;
        let view = d.view();
        assert_eq!(view.activity, Activity::PlayingIn(5));
        assert_eq!(view.playing.as_deref(), Some(LABEL));
        assert!(view.details().contains(&format!("Playing {LABEL}")));
        sleep(secs(5)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Playing);
        assert_eq!(d.shared.calls(), []);
        sleep(secs(1)).await;
        d.settle().await;

        let expected = [
            (ms(5010), w(PressState::Down)),
            (ms(5500), w(PressState::Up)),
        ];
        assert_eq!(d.shared.times_since(start), expected);
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(d.notice(), format!("Finished playing {LABEL}"));
        assert_eq!(d.view().playing, None);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_hotkey_plays_when_the_modifiers_come_up() {
    local(async {
        let (d, _) = daemon_with_hold(playing(once())).await;
        let start = Instant::now();
        d.hotkey(PLAY).await;
        assert_eq!(d.activity(), Activity::PlayWaitingForRelease);
        sleep(secs(1)).await;
        d.settle().await;
        assert_eq!(d.shared.calls(), []);
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        assert_eq!(
            (d.activity(), d.notice()),
            (Activity::Playing, String::new())
        );
        sleep(ms(10)).await;
        d.settle().await;
        assert_eq!(
            d.shared.times_since(start),
            [(ms(1010), w(PressState::Down))]
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_hotkey_still_held_after_two_seconds_plays_anyway_and_says_so() {
    local(async {
        let (d, _) = daemon_with_hold(playing(once())).await;
        let start = Instant::now();
        d.hotkey(PLAY).await;
        sleep(ms(1999)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::PlayWaitingForRelease);
        sleep(ms(1)).await;
        d.settle().await;

        assert_eq!(d.activity(), Activity::Playing);
        assert_eq!(
            d.notice(),
            "Playing although the hotkey was still held after 2 s; its keys mix into what plays"
        );
        // A release that comes now starts nothing more.
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        sleep(ms(600)).await;
        d.settle().await;
        let expected = [
            (ms(2010), w(PressState::Down)),
            (ms(2500), w(PressState::Up)),
        ];
        assert_eq!(d.shared.times_since(start), expected);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_release_handled_only_after_the_two_seconds_says_the_hotkey_was_held() {
    local(async {
        let (d, _) = daemon_with_hold(playing(once())).await;
        d.hotkey(PLAY).await;
        // The controller is late: the release and the deadline arrive
        // together.
        tokio::time::advance(secs(2)).await;
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        assert_eq!(d.activity(), Activity::Playing);
        assert!(d.notice().starts_with("Playing although"), "{}", d.notice());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn everything_that_cancels_a_start_cancels_playback_at_once() {
    use Cancel::*;
    for cancel in [
        Stop,
        Hotkey,
        ReleaseLost,
        Unavailable,
        Locked,
        LockRequested,
        Quit,
    ] {
        local(async {
            let (d, _) = daemon_with_hold(playing(once())).await;
            d.hotkey(PLAY).await;
            let before = Instant::now();
            cancel.send(&d);
            d.settle().await;
            assert_eq!(d.shared.recordings_path().read_dir().unwrap().count(), 1);
            assert_eq!(Instant::now(), before, "{cancel:?}: no time passed");
            if matches!(cancel, Quit) {
                assert!(d.task.is_finished());
            } else {
                assert_eq!(d.activity(), Activity::Idle, "{cancel:?}");
            }
            // Nothing starts later either.
            sleep(secs(5)).await;
            assert_eq!(d.shared.calls(), [], "{cancel:?}");
            let notice = match cancel {
                ReleaseLost => {
                    "Not playing: could not tell when the hotkey was let go; press Ctrl+Shift+F8 again"
                }
                Unavailable => "Not playing: hotkeys stopped working",
                Locked | LockRequested => "Stopped: the screen is locked",
                Stop | Hotkey | Quit => "",
            };
            assert_eq!(d.notice(), notice, "{cancel:?}");
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn the_tray_countdown_is_cancelled_by_stop_and_by_any_hotkey() {
    let fired = |action| HotkeyEvent::Fired(action);
    let cancels = [
        None,
        Some(fired(Action::ToggleClicking)),
        Some(fired(Action::ToggleRecording)),
        Some(PLAY),
    ];
    for cancel in cancels {
        local(async {
            let config = Config {
                tray_start_delay: secs(5),
                ..playing(once())
            };
            let (d, file) = daemon_with_hold(config).await;
            d.send(Message::Play(file)).await;
            match cancel.clone() {
                Some(event) => d.hotkey(event).await,
                None => d.send(Message::Stop).await,
            }
            assert_eq!(d.activity(), Activity::Idle, "{cancel:?}");
            sleep(secs(10)).await;
            d.assert_idle_after(&[]);
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn any_hotkey_cancels_playback_waiting_for_the_modifiers_or_the_devices() {
    for action in [Action::ToggleRecording, Action::TogglePlayback] {
        local(async {
            // Waiting for the modifiers.
            let (d, _) = daemon_with_hold(playing(once())).await;
            d.hotkey(PLAY).await;
            d.hotkey(HotkeyEvent::Fired(action)).await;
            assert_eq!(d.activity(), Activity::Idle, "{action:?}");
            d.hotkey(HotkeyEvent::ModifiersReleased).await;
            sleep(secs(5)).await;
            d.assert_idle_after(&[]);

            // Waiting for the devices to open.
            let (d, gates) = per_run_daemon(Mode::Click).await;
            d.shared
                .with(|r| r.recordings = Some(Arc::new(TempDir::new())));
            let file = d.shared.store_hold();
            d.send(Message::Play(file)).await;
            assert_eq!(d.activity(), Activity::PlayOpening);
            d.hotkey(HotkeyEvent::Fired(action)).await;
            assert_eq!(gates.gates(|g| g.opens_dropped), 1, "{action:?}");
            d.assert_idle_after(&[]);
        })
        .await;
    }
}

// Stopping

#[tokio::test(start_paused = true)]
async fn any_hotkey_stops_playback_mid_press_at_once_and_starts_nothing() {
    for action in [
        Action::ToggleClicking,
        Action::ToggleRecording,
        Action::TogglePlayback,
    ] {
        local(async {
            let (d, _) = daemon_with_hold(playing(Repeat::UntilStopped)).await;
            d.play_until_w_is_down().await;

            let before = Instant::now();
            d.hotkey(HotkeyEvent::Fired(action)).await;
            assert_eq!(Instant::now(), before, "{action:?}: no time passed");
            d.assert_idle_after(&[w(PressState::Down), w(PressState::Up)]);
            // The hotkey only stopped: it started neither clicking nor a
            // recording nor the next play.
            d.hotkey(HotkeyEvent::ModifiersReleased).await;
            sleep(secs(5)).await;
            d.assert_idle_after(&[w(PressState::Down), w(PressState::Up)]);
            assert!(d.shared.recordings_path().read_dir().unwrap().count() == 1);
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn stop_a_lock_sleep_and_quit_release_what_playback_holds_at_once() {
    let stops = [
        (Message::Stop, ""),
        (
            Message::Session(Some(Blocked::Locked)),
            "Stopped: the screen is locked",
        ),
        (
            Message::Session(Some(Blocked::Inactive)),
            "Stopped: another session is in front",
        ),
        (Message::LockRequested, "Stopped: the screen is locked"),
        (Message::Sleep(1), "Stopped: the computer is going to sleep"),
        (Message::Quit, ""),
    ];
    for (stop, notice) in stops {
        local(async {
            let (d, _) = daemon_with_hold(playing(Repeat::UntilStopped)).await;
            d.play_until_w_is_down().await;

            let before = Instant::now();
            d.send(stop.clone()).await;
            assert_eq!(Instant::now(), before, "{stop:?}: no time passed");
            let calls = [w(PressState::Down), w(PressState::Up)];
            assert_eq!(d.shared.calls(), calls, "{stop:?}");
            if stop == Message::Quit {
                assert!(d.task.is_finished());
            } else {
                assert_eq!(d.activity(), Activity::Idle, "{stop:?}");
            }
            assert_eq!(d.notice(), notice, "{stop:?}");
            if stop == Message::Sleep(1) {
                // Reported only after the release.
                assert_eq!(*d.released_for_sleep.borrow(), 1);
                assert_eq!(d.shared.with(|r| r.reported_at_call.clone()), [0, 0]);
            }
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_failed_write_stops_playback_and_says_why() {
    local(async {
        let (d, _) = daemon_with_hold(playing(once())).await;
        d.shared.log(0).fail_once(w(PressState::Down));
        d.hotkey(PLAY).await;
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        sleep(ms(20)).await;
        d.settle().await;

        assert_eq!(d.activity(), Activity::Idle);
        assert!(
            d.notice().starts_with("Playback stopped: "),
            "{}",
            d.notice()
        );
        // The key whose press failed is released: it may have gone down.
        assert_eq!(d.shared.calls(), [w(PressState::Down), w(PressState::Up)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_hotkey_is_read_and_stops_playback_in_the_middle_of_a_dense_burst() {
    local(async {
        let shared = shared_with_recordings();
        let burst: Vec<(u64, Change)> = (0..100_000).map(|_| (0, Change::Wheel(1))).collect();
        let file = shared.store("18:00:00", 1000, &burst);
        // A reader that reads only when the run leaves it budget to: the
        // run hands the thread back between batches before it has spent
        // the task's budget again.
        shared.with(|r| {
            r.busy_readers = true;
            r.pressed_during_a_run = Some(PLAY);
        });
        let d = daemon_with(playing(once()), shared, true).await;

        d.inbox.send(Message::Play(file)).unwrap();
        d.settle_until(|d| d.activity() == Activity::Idle && !d.shared.calls().is_empty())
            .await;

        // Stopped after a batch or two, not after the whole burst.
        let played = d.shared.calls().len();
        assert!((1..1000).contains(&played), "{played} events played");
        assert_eq!(d.notice(), "");
    })
    .await;
}

// Which recording plays

#[tokio::test(start_paused = true)]
async fn the_hotkey_plays_the_newest_then_the_last_chosen_then_the_last_recorded() {
    local(async {
        let shared = shared_with_recordings();
        let older = shared.store_wheel("18:00:00", 1);
        shared.store_wheel("18:01:00", 2);
        let d = daemon_with(playing(once()), shared, true).await;
        let play = || async {
            d.hotkey(PLAY).await;
            d.hotkey(HotkeyEvent::ModifiersReleased).await;
            sleep(secs(1)).await;
            d.settle().await;
            d.shared.calls().last().copied()
        };

        // Nothing recorded or chosen yet: the newest.
        assert_eq!(play().await, Some(Call::Scroll(2)));
        // The one chosen from the tray, from then on.
        d.send(Message::Play(older)).await;
        sleep(secs(1)).await;
        assert_eq!(play().await, Some(Call::Scroll(1)));
        // The one just recorded.
        d.record_hotkey().await;
        sleep(ms(100)).await;
        d.key(KEYBOARD, W, true);
        sleep(ms(100)).await;
        d.key(KEYBOARD, W, false);
        d.record_hotkey().await;
        assert_eq!(play().await, Some(w(PressState::Up)));
        assert_eq!(d.notice(), "Finished playing Recording 2026-11-09 18:04:05");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn when_the_current_recording_is_gone_the_hotkey_plays_the_newest() {
    local(async {
        let shared = shared_with_recordings();
        let older = shared.store_wheel("18:00:00", 1);
        let newer = shared.store_wheel("18:01:00", 2);
        let d = daemon_with(playing(once()), shared, true).await;
        d.send(Message::Play(older.clone())).await;
        sleep(secs(1)).await;
        fs::remove_file(d.shared.recordings_path().join(older.as_str())).unwrap();

        d.hotkey(PLAY).await;
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        sleep(secs(1)).await;
        d.settle().await;

        assert_eq!(d.shared.calls(), [Call::Scroll(1), Call::Scroll(2)]);
        assert_eq!(d.listed(), [newer.label()]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_damaged_current_recording_is_reported_and_no_other_plays_in_its_place() {
    local(async {
        let shared = shared_with_recordings();
        let chosen = shared.store_wheel("18:00:00", 1);
        shared.store_wheel("18:01:00", 2);
        let d = daemon_with(playing(Repeat::UntilStopped), shared, true).await;
        d.send(Message::Play(chosen.clone())).await;
        d.send(Message::Stop).await;
        let path = d.shared.recordings_path().join(chosen.as_str());

        // Damaged: said so, and the newer recording does not play instead.
        fs::write(&path, b"").unwrap();
        d.hotkey(PLAY).await;
        assert_eq!(
            (d.activity(), d.notice().contains(LABEL)),
            (Activity::Idle, true)
        );
        // Too short to repeat: refused, and it stays the current one.
        let made = Created::parse("2026-11-09T18:00:00.000+01:00").unwrap();
        let short = Recording::new(made.default_name(), made, 1000, Vec::new()).unwrap();
        let mut json = Vec::new();
        short.write_json(&mut json).unwrap();
        fs::write(&path, json).unwrap();
        for _ in 0..2 {
            d.hotkey(PLAY).await;
            let refused = format!("Not playing {LABEL}: it is shorter than 100 ms");
            assert!(d.notice().starts_with(&refused), "{}", d.notice());
        }
        d.assert_idle_after(&[Call::Scroll(1)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn with_no_recordings_the_hotkey_says_there_is_nothing_to_play() {
    local(async {
        let d = daemon_with(playing(once()), Shared::default(), true).await;
        d.hotkey(PLAY).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(d.notice(), "Nothing to play: no recordings yet");

        // No data directory at all reads the same.
        d.shared.with(|r| r.recordings = None);
        d.hotkey(PLAY).await;
        assert_eq!(d.notice(), "Nothing to play: no recordings yet");
        d.assert_idle_after(&[]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_missing_or_invalid_file_says_why_and_presses_nothing() {
    local(async {
        let config = Config {
            tray_start_delay: secs(5),
            ..playing(once())
        };
        let (d, file) = daemon_with_hold(config).await;
        let path = d.shared.recordings_path().join(file.as_str());

        // Half a file.
        fs::write(&path, b"{\"version\":1,\"name\":\"x\"").unwrap();
        d.send(Message::Play(file.clone())).await;
        assert_eq!(
            (d.activity(), d.notice()),
            (
                Activity::Idle,
                format!("Not playing {LABEL}: the file ends too early (line 1, column 23)")
            )
        );
        // A recording that leaves a key down.
        fs::write(
            &path,
            br#"{"version":1,"name":"x","created":"2026-11-09T18:00:00.000+01:00","duration_us":1000,"events":[[0,"press","KEY_W"]]}"#,
        )
        .unwrap();
        d.hotkey(PLAY).await;
        assert_eq!(
            d.notice(),
            format!("Not playing {LABEL}: the recording ends with a key or button still down")
        );
        // No file.
        fs::remove_file(&path).unwrap();
        d.send(Message::Play(file)).await;
        assert_eq!(
            d.notice(),
            format!("Not playing {LABEL}: the file is no longer there")
        );
        // The list no longer offers it.
        assert_eq!(d.listed(), [""; 0]);

        sleep(secs(10)).await;
        d.assert_idle_after(&[]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_too_short_to_repeat_is_refused() {
    for repeat in [
        Repeat::UntilStopped,
        Repeat::Times(NonZeroU32::new(2).unwrap()),
    ] {
        local(async {
            let shared = shared_with_recordings();
            let file = shared.store("18:00:00", 99, &[(0, Change::Wheel(1))]);
            let d = daemon_with(playing(repeat), shared, true).await;
            let refused = format!(
                "Not playing {LABEL}: it is shorter than 100 ms, too short to play more than once"
            );

            d.send(Message::Play(file)).await;
            assert_eq!(
                (d.activity(), d.notice()),
                (Activity::Idle, refused.clone())
            );
            d.hotkey(PLAY).await;
            assert_eq!((d.activity(), d.notice()), (Activity::Idle, refused));
            sleep(secs(5)).await;
            d.assert_idle_after(&[]);
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn playback_repeats_and_runs_at_the_configured_speed() {
    local(async {
        let config = Config {
            playback: PlaybackSettings {
                speed: Speed::from_ratio(2.0).unwrap(),
                repeat: Repeat::Times(NonZeroU32::new(2).unwrap()),
            },
            ..playing(once())
        };
        let (d, file) = daemon_with_hold(config).await;
        let start = Instant::now();
        d.send(Message::Play(file)).await;
        sleep(secs(2)).await;
        d.settle().await;

        let (down, up) = (w(PressState::Down), w(PressState::Up));
        let expected = [(ms(5), down), (ms(250), up), (ms(505), down), (ms(750), up)];
        assert_eq!(d.shared.times_since(start), expected);
        assert_eq!(d.notice(), format!("Finished playing {LABEL}"));
    })
    .await;
}

// What is refused

#[tokio::test(start_paused = true)]
async fn playback_is_refused_while_clicking_and_ignored_while_recording() {
    local(async {
        let (d, file) = daemon_with_hold(playing(once())).await;
        // Clicking, or about to.
        d.press_hotkey().await;
        for _ in 0..2 {
            let activity = d.activity();
            d.hotkey(PLAY).await;
            assert_eq!(d.notice(), "Not playing: clicking is on; stop it first");
            d.send(Message::Play(file.clone())).await;
            assert_eq!(d.activity(), activity);
            d.hotkey(HotkeyEvent::ModifiersReleased).await;
        }
        assert_eq!(d.activity(), Activity::Clicking);
        d.send(Message::Stop).await;

        // About to record, from the tray.
        let counting = Config {
            tray_start_delay: secs(5),
            ..playing(once())
        };
        let (c, counted) = daemon_with_hold(counting).await;
        c.send(Message::Record).await;
        c.hotkey(PLAY).await;
        c.send(Message::Play(counted)).await;
        assert_eq!(
            (c.activity(), c.notice()),
            (Activity::RecordingIn(5), String::new())
        );

        // Recording.
        d.record_hotkey().await;
        d.hotkey(PLAY).await;
        d.send(Message::Play(file)).await;
        assert_eq!(
            (d.activity(), d.notice()),
            (Activity::Recording, String::new())
        );
        assert_eq!(d.shared.calls(), [LEFT_DOWN, LEFT_UP]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn clicking_and_recording_from_the_tray_do_not_start_while_playing() {
    local(async {
        let (d, file) = daemon_with_hold(playing(Repeat::UntilStopped)).await;
        d.play_until_w_is_down().await;
        for message in [Message::Start, Message::Record, Message::Play(file)] {
            d.send(message).await;
            assert_eq!(d.activity(), Activity::Playing);
        }
        assert_eq!(d.shared.calls(), [w(PressState::Down)]);
        // The tray's one stop ends it.
        d.send(Message::Stop).await;
        d.assert_idle_after(&[w(PressState::Down), w(PressState::Up)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_blocked_session_refuses_playback_before_reading_the_file() {
    local(async {
        let (d, file) = daemon_with_hold(playing(once())).await;
        d.send(Message::Session(Some(Blocked::Locked))).await;
        // An unreadable file would say so, were it read.
        fs::write(d.shared.recordings_path().join(file.as_str()), b"").unwrap();
        for start in [None, Some(file)] {
            match start {
                Some(file) => d.send(Message::Play(file)).await,
                None => d.hotkey(PLAY).await,
            }
            assert_eq!(d.notice(), "Not playing: the screen is locked");
        }
        d.assert_idle_after(&[]);
    })
    .await;
}

// The list

#[tokio::test(start_paused = true)]
async fn the_newest_ten_are_listed_at_start_after_a_save_and_when_the_menu_opens() {
    local(async {
        let shared = shared_with_recordings();
        for minute in 0..12 {
            shared.store_wheel(&format!("17:{minute:02}:00"), 1);
        }
        let d = daemon_with(playing(once()), shared, true).await;
        let label = |time: &str| format!("Recording 2026-11-09 {time}");

        let listed = d.listed();
        assert_eq!(listed.len(), 10);
        assert_eq!(
            (listed[0].clone(), listed[9].clone()),
            (label("17:11:00"), label("17:02:00"))
        );

        // A file that appears by hand shows when the menu opens.
        d.shared.store_wheel("17:30:00", 1);
        assert_eq!(d.listed()[0], label("17:11:00"));
        d.send(Message::MenuOpened).await;
        assert_eq!(d.listed()[0], label("17:30:00"));

        // A saved recording shows at once.
        d.record_hotkey().await;
        d.key(KEYBOARD, W, true);
        d.key(KEYBOARD, W, false);
        d.record_hotkey().await;
        assert_eq!(d.listed()[0], label("18:04:05"));
        assert_eq!(d.listed().len(), 10);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_folder_is_not_read_for_the_menu_while_something_runs() {
    local(async {
        let (d, _) = daemon_with_hold(playing(Repeat::UntilStopped)).await;
        d.play_until_w_is_down().await;
        d.shared.store_wheel("19:00:00", 1);
        d.send(Message::MenuOpened).await;
        assert_eq!(d.listed(), [LABEL]);
        d.send(Message::Stop).await;
        d.send(Message::MenuOpened).await;
        assert_eq!(d.listed(), ["Recording 2026-11-09 19:00:00", LABEL]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_folder_that_cannot_be_listed_is_a_problem_not_an_empty_list() {
    local(async {
        let (d, _) = daemon_with_hold(playing(once())).await;
        let folder = d.shared.recordings_path();
        fs::remove_dir_all(&folder).unwrap();
        fs::write(&folder, b"").unwrap();

        d.send(Message::MenuOpened).await;
        let problems = d.problems();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].starts_with("The recordings cannot be listed: "),
            "{problems:?}"
        );
        assert_eq!(d.listed(), [""; 0]);
        d.hotkey(PLAY).await;
        assert!(
            d.notice()
                .starts_with("Not playing: the recordings cannot be listed: "),
            "{}",
            d.notice()
        );

        // Once it can be listed again, the problem goes.
        fs::remove_file(&folder).unwrap();
        fs::create_dir(&folder).unwrap();
        d.send(Message::MenuOpened).await;
        assert_eq!(d.problems(), [""; 0]);
    })
    .await;
}

// Devices that open for each run

#[tokio::test(start_paused = true)]
async fn playback_opens_the_devices_for_its_run_and_closes_them_after() {
    local(async {
        let (d, gates) = per_run_daemon(Mode::Click).await;
        d.shared
            .with(|r| r.recordings = Some(Arc::new(TempDir::new())));
        let file = d.shared.store_hold();

        d.send(Message::Play(file.clone())).await;
        assert_eq!(d.activity(), Activity::PlayOpening);
        assert_eq!(d.view().playing.as_deref(), Some(LABEL));
        // A stop drops the open, the desktop's dialog with it.
        d.send(Message::Stop).await;
        assert_eq!(gates.gates(|g| g.opens_dropped), 1);
        d.assert_idle_after(&[]);

        // An open that fails says why playback did not start.
        d.send(Message::Play(file.clone())).await;
        gates.finish_open(Err(InputError::NotAllowed));
        d.settle().await;
        assert!(d.notice().starts_with("Not playing: "), "{}", d.notice());
        d.assert_idle_after(&[]);

        d.send(Message::Play(file)).await;
        gates.finish_open(Ok(()));
        d.settle().await;
        assert_eq!(d.activity(), Activity::Playing);
        sleep(secs(1)).await;
        d.settle().await;
        d.assert_idle_after(&[w(PressState::Down), w(PressState::Up)]);
        assert_eq!(gates.gates(|g| g.closed), 1, "closed after the run");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn losing_the_devices_stops_playback_mid_press_at_once() {
    local(async {
        let (d, gates) = per_run_daemon(Mode::Click).await;
        d.shared
            .with(|r| r.recordings = Some(Arc::new(TempDir::new())));
        let file = d.shared.store_hold();
        d.send(Message::Play(file)).await;
        gates.finish_open(Ok(()));
        sleep(ms(20)).await;
        d.settle().await;
        assert_eq!(d.shared.calls(), [w(PressState::Down)]);

        let before = Instant::now();
        gates.lose(InputError::SessionLost("disconnected".to_owned()));
        d.settle().await;
        assert_eq!(Instant::now(), before, "no time passed");
        d.assert_idle_after(&[w(PressState::Down), w(PressState::Up)]);
        assert!(
            d.notice()
                .starts_with("Playback stopped: the desktop ended remote input"),
            "{}",
            d.notice()
        );
        assert_eq!(gates.gates(|g| g.closed), 1);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn quitting_mid_press_releases_and_closes_the_devices_at_once() {
    local(async {
        let (d, gates) = per_run_daemon(Mode::Click).await;
        d.shared
            .with(|r| r.recordings = Some(Arc::new(TempDir::new())));
        let file = d.shared.store_hold();
        d.send(Message::Play(file)).await;
        gates.finish_open(Ok(()));
        sleep(ms(20)).await;
        d.settle().await;
        assert_eq!(d.shared.calls(), [w(PressState::Down)]);

        let before = Instant::now();
        d.send(Message::Quit).await;
        assert_eq!(Instant::now(), before, "no time passed");
        assert!(d.task.is_finished());
        assert_eq!(d.shared.calls(), [w(PressState::Down), w(PressState::Up)]);
        assert_eq!(gates.gates(|g| g.closed), 1);
    })
    .await;
}
