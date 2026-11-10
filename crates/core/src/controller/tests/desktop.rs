//! Hotkeys the desktop delivers: the controller never presses a hotkey's
//! key, shows the triggers the desktop reports, and keeps the reader running
//! through every open.

use super::*;
use crate::hotkeys::{Trigger, Triggers};
use crate::recording::{Change, recordable};

const SPACE_DOWN: Call = Call::Key(space(), PressState::Down);

const fn space() -> Key {
    match Key::new(57) {
        Some(key) => key,
        None => panic!("57 is KEY_SPACE"),
    }
}

/// A daemon whose reader gets its hotkeys from the desktop, started from
/// the tray without a countdown.
async fn desktop_daemon(clicker: ClickerSettings) -> Daemon {
    let shared = Shared::default();
    shared.with(|r| r.desktop_reader = true);
    let config = Config {
        tray_start_delay: Duration::ZERO,
        clicker,
        ..Config::default()
    };
    daemon_with(config, shared, true).await
}

fn pressing(mode: Mode, key: &str) -> ClickerSettings {
    ClickerSettings {
        mode,
        key: Key::from_name(key).unwrap(),
        ..ClickerSettings::default()
    }
}

fn reported(clicking: &str, recording: &str, playback: &str) -> HotkeyEvent {
    let mut triggers = Triggers::unknown();
    triggers.set(Action::ToggleClicking, Trigger::reported(clicking));
    triggers.set(Action::ToggleRecording, Trigger::reported(recording));
    triggers.set(Action::TogglePlayback, Trigger::reported(playback));
    HotkeyEvent::Triggers(triggers)
}

fn defaults() -> HotkeyEvent {
    reported("Ctrl+Shift+F6", "Ctrl+Shift+F7", "Ctrl+Shift+F8")
}

#[tokio::test(start_paused = true)]
async fn a_run_that_would_press_a_hotkeys_key_never_starts() {
    // The program's own output of a bound combination's key would set the
    // hotkey off, since the desktop cannot tell whose key it is. So no such
    // key is ever pressed: neither from the tray nor from the hotkey.
    for mode in [Mode::KeyRepeat, Mode::HoldKey] {
        for key in ["f6", "f7", "f8"] {
            local(async {
                let d = desktop_daemon(pressing(mode, key)).await;
                d.hotkey(defaults()).await;
                d.send(Message::Start).await;
                assert_eq!(d.activity(), Activity::Idle, "{mode:?} {key}");
                let notice = d.notice();
                assert!(
                    notice.starts_with(&format!("Not started: {key} is the key of the hotkey")),
                    "{notice}"
                );
                d.press_hotkey().await;
                d.hotkey(HotkeyEvent::ModifiersReleased).await;
                assert_eq!(d.activity(), Activity::Idle, "{mode:?} {key}");
                sleep(secs(60)).await;
                assert_eq!(d.shared.calls(), [], "{mode:?} {key}");
            })
            .await;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_key_run_waits_for_the_desktop_to_report_readable_triggers() {
    local(async {
        let d = desktop_daemon(pressing(Mode::HoldKey, "space")).await;
        // Nothing reported yet: the key might be a hotkey's.
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert!(d.notice().contains("has not said yet"), "{}", d.notice());
        // A trigger the program cannot read rules key runs out too.
        d.hotkey(reported("Ctrl+Shift+F6", "Ctrl+Space", "")).await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert!(d.notice().contains("Ctrl+Space"), "{}", d.notice());
        assert_eq!(d.shared.calls(), []);
        // Readable triggers on other keys: the run starts.
        d.hotkey(defaults()).await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Clicking);
        assert_eq!(d.shared.calls(), [SPACE_DOWN]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn key_runs_are_allowed_while_the_desktop_delivers_no_hotkeys() {
    local(async {
        // The bind never succeeded: no session, so nothing can fire, and
        // the tray must still be able to start a key run.
        let d = desktop_daemon(pressing(Mode::HoldKey, "f6")).await;
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Unavailable(
            Unavailable::Desktop("no portal".to_owned()),
        )))
        .await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Clicking);
        // The desktop comes back and reports a trigger on the run's key:
        // the run stops before any hotkey of the new session is handled.
        let start = Instant::now();
        d.hotkey(defaults()).await;
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Available)).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(start.elapsed(), Duration::ZERO);
        assert!(
            d.notice()
                .starts_with("Stopped: f6 is the key of the hotkey"),
            "{}",
            d.notice()
        );
        // And it is refused again from here on.
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Idle);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_start_between_the_new_triggers_and_available_is_still_refused() {
    local(async {
        let d = desktop_daemon(pressing(Mode::HoldKey, "f6")).await;
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Unavailable(
            Unavailable::Desktop("the portal stopped".to_owned()),
        )))
        .await;
        // The new session's triggers arrived; "available" has not yet.
        d.hotkey(defaults()).await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(d.shared.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn other_hotkey_trouble_does_not_lift_the_refusal() {
    local(async {
        let d = desktop_daemon(pressing(Mode::HoldKey, "f6")).await;
        d.hotkey(defaults()).await;
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Unavailable(
            Unavailable::NoKeyboard,
        )))
        .await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Idle);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_modifier_as_the_clicker_key_is_refused() {
    local(async {
        let d = desktop_daemon(pressing(Mode::HoldKey, "leftshift")).await;
        d.hotkey(defaults()).await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert!(d.notice().contains("is a modifier"), "{}", d.notice());
        assert_eq!(d.shared.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn click_modes_start_whatever_the_triggers_are() {
    local(async {
        let d = desktop_daemon(ClickerSettings {
            mode: Mode::HoldButton,
            ..ClickerSettings::default()
        })
        .await;
        // Before any report, and with an unreadable one.
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Clicking);
        d.hotkey(reported("Ctrl+Space", "", "")).await;
        assert_eq!(d.activity(), Activity::Clicking);
        assert_eq!(d.shared.calls(), [LEFT_DOWN]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_trigger_moved_onto_the_runs_key_stops_the_run_at_once() {
    local(async {
        let d = desktop_daemon(pressing(Mode::HoldKey, "f9")).await;
        d.hotkey(defaults()).await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Clicking);
        let start = Instant::now();
        // The user rebinds recording to Ctrl+F9 in System Settings.
        d.hotkey(reported("Ctrl+Shift+F6", "Ctrl+F9", "Ctrl+Shift+F8"))
            .await;
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(start.elapsed(), Duration::ZERO);
        assert!(
            d.notice()
                .starts_with("Stopped: f9 is the key of the hotkey"),
            "{}",
            d.notice()
        );
        let f9 = Key::from_name("f9").unwrap();
        assert_eq!(
            d.shared.calls(),
            [
                Call::Key(f9, PressState::Down),
                Call::Key(f9, PressState::Up)
            ]
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_trigger_moved_onto_the_runs_key_cancels_a_pending_start() {
    local(async {
        let d = desktop_daemon(pressing(Mode::KeyRepeat, "f9")).await;
        d.hotkey(defaults()).await;
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
        d.hotkey(reported("Ctrl+F9", "", "")).await;
        assert_eq!(d.activity(), Activity::Idle);
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(d.shared.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_trigger_moved_onto_the_runs_key_cancels_a_tray_countdown() {
    local(async {
        let shared = Shared::default();
        shared.with(|r| r.desktop_reader = true);
        let config = Config {
            clicker: pressing(Mode::KeyRepeat, "f9"),
            ..Config::default()
        };
        let d = daemon_with(config, shared, true).await;
        d.hotkey(defaults()).await;
        d.send(Message::Start).await;
        assert!(matches!(d.activity(), Activity::StartingIn(_)));
        d.hotkey(reported("Ctrl+F9", "", "")).await;
        assert_eq!(d.activity(), Activity::Idle);
        sleep(secs(60)).await;
        assert_eq!(d.shared.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_trigger_change_leaves_other_runs_and_idle_alone() {
    local(async {
        let d = desktop_daemon(pressing(Mode::HoldKey, "f9")).await;
        d.hotkey(defaults()).await;
        // Idle: a colliding report says nothing by itself.
        d.hotkey(reported("Ctrl+F9", "", "")).await;
        assert_eq!(d.notice(), "");
        // A run on a free key keeps going through an unrelated change.
        d.hotkey(defaults()).await;
        d.send(Message::Start).await;
        d.hotkey(reported("Ctrl+Shift+F6", "Meta+F7", "")).await;
        assert_eq!(d.activity(), Activity::Clicking);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_tray_shows_the_triggers_the_desktop_reports() {
    local(async {
        let d = desktop_daemon(ClickerSettings::default()).await;
        // Not the config's bindings: the desktop decides the keys.
        assert_eq!(d.view().hotkey, Hotkey::Desktop(None));
        let lines = d.view().details();
        assert!(!lines.iter().any(|line| line.contains("F6")), "{lines:?}");
        d.hotkey(reported("Ctrl+Shift+F6", "", "Ctrl+Shift+F8"))
            .await;
        let lines = d.view().details();
        assert_eq!(lines[1], "Ctrl+Shift+F6 starts and stops clicking");
        assert!(
            lines[2].starts_with("No hotkey starts recording"),
            "{lines:?}"
        );
        // The user changes one in System Settings.
        d.hotkey(reported("Meta+F11", "Ctrl+Shift+F7", "")).await;
        let lines = d.view().details();
        assert_eq!(lines[1], "Meta+F11 starts and stops clicking");
        assert_eq!(
            lines[2],
            "Ctrl+Shift+F7 starts recording, and stops and saves it"
        );
        d.press_hotkey().await;
        assert_eq!(
            d.view().title(),
            "Auto clicker: starts 1 s after you let go of the hotkey"
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_desktops_reader_runs_through_every_open() {
    local(async {
        let d = desktop_daemon(ClickerSettings {
            mode: Mode::HoldButton,
            ..ClickerSettings::default()
        })
        .await;
        d.send(Message::Start).await;
        // The devices fail and are opened again: the reader, and with it the
        // desktop's session, is neither dropped nor started twice.
        d.shared.log(0).fail_once(LEFT_UP);
        d.send(Message::Stop).await;
        assert_eq!(
            d.shared.steps(),
            [Step::ReaderStarted(Some(0)), Step::Opened(Some(1))]
        );
        // It still delivers hotkeys, and the next run uses the new devices.
        d.start_by_hotkey().await;
        assert_eq!(d.shared.calls_to(1), [LEFT_DOWN]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn losing_the_desktops_hotkeys_stops_neither_a_run_nor_the_tray() {
    local(async {
        let d = desktop_daemon(ClickerSettings {
            mode: Mode::HoldButton,
            ..ClickerSettings::default()
        })
        .await;
        d.hotkey(defaults()).await;
        d.send(Message::Start).await;
        let lost = Unavailable::Desktop("the desktop's shortcuts service stopped".to_owned());
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Unavailable(lost.clone())))
            .await;
        assert_eq!(d.activity(), Activity::Clicking);
        assert_eq!(d.view().problems, [lost.to_string()]);
        assert_eq!(
            lost.to_string(),
            "hotkeys are unavailable: the desktop's shortcuts service stopped"
        );
        // The tray still stops and starts it.
        d.send(Message::Stop).await;
        assert_eq!(d.shared.calls(), [LEFT_DOWN, LEFT_UP]);
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Clicking);
        // The hotkeys come back: the problem goes.
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Available)).await;
        assert_eq!(d.view().problems, [] as [String; 0]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn losing_the_desktops_hotkeys_cancels_a_start_waiting_for_the_release() {
    local(async {
        let d = desktop_daemon(ClickerSettings::default()).await;
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
        let start = Instant::now();
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Unavailable(
            Unavailable::Desktop("gone".to_owned()),
        )))
        .await;
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(start.elapsed(), Duration::ZERO);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn recording_is_refused_while_the_desktop_delivers_the_hotkeys() {
    local(async {
        let d = desktop_daemon(ClickerSettings::default()).await;
        d.hotkey(defaults()).await;
        for start in [
            Message::Record,
            // The recording hotkey, as the desktop delivers it.
            Message::MenuOpened,
        ] {
            if start == Message::MenuOpened {
                d.hotkey(HotkeyEvent::Fired(Action::ToggleRecording)).await;
            } else {
                d.send(start).await;
            }
            assert_eq!(d.activity(), Activity::Idle);
            let notice = d.notice();
            assert!(
                notice.starts_with("Not recording: recording reads the keyboards")
                    && notice.contains("hotkeys = \"kernel\""),
                "{notice}"
            );
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_hotkey_set_off_by_the_programs_own_keys_as_the_desktop_returns_does_nothing() {
    local(async {
        // The shortcuts were away, so the run may press F8. The desktop
        // binds them again; the run's next F8 sets off the playback hotkey
        // before the program has heard of the new triggers.
        let d = desktop_daemon_with_recording(&["w"]).await;
        d.hotkey(defaults()).await;
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Unavailable(
            Unavailable::Desktop("a portal service of the desktop stopped".to_owned()),
        )))
        .await;
        d.hotkey(PLAY).await;
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        sleep(Duration::from_millis(20)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Playing);
        d.send(Message::Stop).await;
        let calls = d.shared.calls().len();
        // Back: the triggers, then the echo of every hotkey.
        d.hotkey(defaults()).await;
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Available)).await;
        for action in Action::ALL {
            d.hotkey(HotkeyEvent::Fired(action)).await;
            assert_eq!(d.activity(), Activity::Idle, "{action:?}");
        }
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        // Still nothing just before the window closes.
        sleep(Duration::from_millis(999)).await;
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(d.shared.calls().len(), calls);
        // After it, a hotkey is the user's again.
        sleep(Duration::from_millis(1)).await;
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_trigger_report_during_the_echo_window_leaves_it_open() {
    local(async {
        let d = desktop_daemon(ClickerSettings::default()).await;
        d.hotkey(defaults()).await;
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Unavailable(
            Unavailable::Desktop("a portal service of the desktop stopped".to_owned()),
        )))
        .await;
        d.hotkey(defaults()).await;
        sleep(Duration::from_millis(400)).await;
        // A second report of the same session, before the echo.
        d.hotkey(reported("Ctrl+Shift+F6", "Meta+F7", "")).await;
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::Idle);
        // The window still ends a second after the return, not later.
        sleep(Duration::from_millis(600)).await;
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_first_triggers_and_a_change_of_them_open_no_echo_window() {
    local(async {
        // Nothing could be pressed before the first report, and a change in
        // System Settings is no return: the user's hotkey works at once.
        let d = desktop_daemon(ClickerSettings::default()).await;
        d.hotkey(defaults()).await;
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
        d.press_hotkey().await;
        d.hotkey(reported("Ctrl+Shift+F6", "Meta+F7", "")).await;
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
    })
    .await;
}

// Playback

const PLAY: HotkeyEvent = HotkeyEvent::Fired(Action::TogglePlayback);

/// A desktop daemon with one recording stored: `name` pressed at 10 ms and
/// let go at 500 ms, in a recording of a second.
async fn desktop_daemon_with_recording(names: &[&str]) -> Daemon {
    let shared = Shared::default();
    shared.with(|r| {
        r.desktop_reader = true;
        r.recordings = Some(Arc::new(TempDir::new()));
    });
    let held: Vec<_> = names
        .iter()
        .map(|name| recordable(Key::from_name(name).unwrap().code()).unwrap())
        .collect();
    let presses = (10..)
        .zip(&held)
        .map(|(at, held)| (at, Change::Press(*held)));
    let releases = (500..)
        .zip(&held)
        .map(|(at, held)| (at, Change::Release(*held)));
    let events: Vec<(u64, Change)> = presses.chain(releases).collect();
    shared.store("18:00:00", 1000, &events);
    let config = Config {
        tray_start_delay: Duration::ZERO,
        ..Config::default()
    };
    daemon_with(config, shared, true).await
}

#[tokio::test(start_paused = true)]
async fn a_recording_that_presses_a_hotkeys_key_is_not_played() {
    // Played back, Ctrl+Shift+F6 would stop its own playback: any hotkey
    // does. Neither the hotkey nor the tray plays it, and nothing is pressed.
    local(async {
        let d = desktop_daemon_with_recording(&["leftctrl", "leftshift", "f6"]).await;
        d.hotkey(defaults()).await;
        d.hotkey(PLAY).await;
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        assert_eq!(d.activity(), Activity::Idle);
        let notice = d.notice();
        assert!(
            notice.starts_with("Not playing Recording 2026-11-09 18:00:00: it presses a hotkey's key: f6 is the key of the hotkey Ctrl+Shift+F6"),
            "{notice}"
        );
        let file = d.view().recordings[0].file.clone();
        d.send(Message::Play(file)).await;
        assert_eq!(d.activity(), Activity::Idle);
        sleep(secs(5)).await;
        assert_eq!(d.shared.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_is_not_played_before_the_triggers_are_known_or_readable() {
    local(async {
        let d = desktop_daemon_with_recording(&["w"]).await;
        d.hotkey(PLAY).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert!(d.notice().contains("has not said yet"), "{}", d.notice());
        d.hotkey(reported("Ctrl+Shift+F6", "Ctrl+Space", "")).await;
        d.hotkey(PLAY).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert!(d.notice().contains("Ctrl+Space"), "{}", d.notice());
        assert_eq!(d.shared.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_that_holds_modifiers_and_other_keys_plays() {
    local(async {
        // Sprinting and sneaking are what the user recorded.
        let d = desktop_daemon_with_recording(&["leftctrl", "leftshift", "w"]).await;
        d.hotkey(defaults()).await;
        d.hotkey(PLAY).await;
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        sleep(Duration::from_millis(20)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Playing);
        assert_eq!(d.shared.calls().len(), 3);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_without_keys_plays_whatever_the_triggers_are() {
    local(async {
        let d = desktop_daemon_with_recording(&[]).await;
        // Nothing reported yet.
        d.hotkey(PLAY).await;
        assert_eq!(d.activity(), Activity::PlayWaitingForRelease);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_trigger_moved_onto_a_key_of_the_recording_stops_playback_at_once() {
    local(async {
        let d = desktop_daemon_with_recording(&["f9"]).await;
        d.hotkey(defaults()).await;
        d.hotkey(PLAY).await;
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        sleep(Duration::from_millis(20)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Playing);
        let start = Instant::now();
        d.hotkey(reported("Ctrl+Shift+F6", "Ctrl+F9", "")).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(start.elapsed(), Duration::ZERO);
        assert!(
            d.notice()
                .starts_with("Stopped: f9 is the key of the hotkey Ctrl+F9"),
            "{}",
            d.notice()
        );
        let f9 = Key::from_name("f9").unwrap();
        assert_eq!(
            d.shared.calls(),
            [
                Call::Key(f9, PressState::Down),
                Call::Key(f9, PressState::Up)
            ]
        );
        // And while it only waits to start.
        d.hotkey(defaults()).await;
        d.hotkey(PLAY).await;
        assert_eq!(d.activity(), Activity::PlayWaitingForRelease);
        d.hotkey(reported("Ctrl+F9", "", "")).await;
        assert_eq!(d.activity(), Activity::Idle);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_with_a_hotkeys_key_plays_where_the_keyboards_are_read() {
    local(async {
        // The device reader leaves the program's own devices out.
        let shared = Shared::default();
        shared.with(|r| r.recordings = Some(Arc::new(TempDir::new())));
        let f6 = recordable(64).unwrap();
        shared.store(
            "18:00:00",
            1000,
            &[(10, Change::Press(f6)), (500, Change::Release(f6))],
        );
        let config = Config {
            tray_start_delay: Duration::ZERO,
            ..Config::default()
        };
        let d = daemon_with(config, shared, true).await;
        d.hotkey(PLAY).await;
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        sleep(Duration::from_millis(20)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Playing);
    })
    .await;
}
