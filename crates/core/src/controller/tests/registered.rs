//! Hotkeys Windows delivers for the bindings the program registered: the
//! controller refuses key runs until it hears which keys they use, opens no
//! echo window for that first report, tells the reader what to read the
//! keyboard for and when to stop, and refuses a hotkey start into a window
//! running as administrator.

use super::*;
use crate::hotkeys::{NotRegistered, Triggers, Unregistered, Watching};

const START: HotkeyEvent = HotkeyEvent::Fired(Action::ToggleClicking);

/// A daemon whose reader reports the triggers of the bindings it
/// registered, as the Windows reader does.
async fn registered_daemon(config: Config, prepare: impl FnOnce(&mut Record)) -> Daemon {
    let shared = Shared::default();
    shared.with(|r| {
        r.registered_triggers = true;
        prepare(r);
    });
    daemon_with(config, shared, true).await
}

fn reported() -> HotkeyEvent {
    HotkeyEvent::Triggers(Triggers::bound(&Bindings::default(), |b| vec![b.key()]))
}

const WAIT: Watching = Watching {
    release: true,
    stops: Vec::new(),
};

fn stopping(actions: &[Action]) -> Watching {
    Watching {
        release: false,
        stops: actions.to_vec(),
    }
}

fn key_press(key: &str) -> Config {
    Config {
        tray_start_delay: Duration::ZERO,
        clicker: ClickerSettings {
            mode: Mode::HoldKey,
            key: Key::from_name(key).unwrap(),
            ..ClickerSettings::default()
        },
        ..Config::default()
    }
}

#[tokio::test(start_paused = true)]
async fn no_key_is_pressed_before_the_bindings_are_reported_and_then_only_theirs_are_refused() {
    local(async {
        let d = registered_daemon(key_press("w"), |_| {}).await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert!(d.notice().contains("has not said yet"), "{}", d.notice());
        d.hotkey(reported()).await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Clicking);
        d.send(Message::Stop).await;
        // The tray names the binding as the config has it, not as a
        // desktop's shortcut.
        assert_eq!(
            d.view().hotkey,
            Hotkey::Bound(Bindings::default().toggle_clicking)
        );
    })
    .await;
    local(async {
        let d = registered_daemon(key_press("f6"), |_| {}).await;
        d.hotkey(reported()).await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert!(d.notice().contains("Ctrl+Shift+F6"), "{}", d.notice());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_first_report_opens_no_echo_window() {
    local(async {
        let d = registered_daemon(config(Mode::HoldButton), |_| {}).await;
        d.hotkey(reported()).await;
        // At once, well inside the second an echo window would last.
        d.hotkey(START).await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        assert_eq!(d.activity(), Activity::Clicking);
    })
    .await;
}

/// The ways a start waiting for the modifiers ends without starting.
#[derive(Clone, Copy, Debug)]
enum WayOut {
    Stop,
    Locked,
    Sleep,
    TimedOut,
    Quit,
}

#[tokio::test(start_paused = true)]
async fn the_reader_reads_for_the_wait_and_every_way_out_ends_it_at_once() {
    use WayOut::*;
    for way in [Stop, Locked, Sleep, TimedOut, Quit] {
        local(async {
            let d = registered_daemon(config(Mode::HoldButton), |_| {}).await;
            d.hotkey(reported()).await;
            d.hotkey(START).await;
            assert_eq!(d.watched().await, [WAIT], "{way:?}");
            sleep(secs(1)).await;
            let before = Instant::now();
            match way {
                Stop => d.send(Message::Stop).await,
                Locked => d.send(Message::Session(Some(Blocked::Locked))).await,
                Sleep => d.send(Message::Sleep(1)).await,
                TimedOut => {
                    sleep(RELEASE_WAIT).await;
                    d.settle().await;
                }
                Quit => d.send(Message::Quit).await,
            }
            if let Quit = way {
                // The reader is dropped with the controller, which ends all
                // it read for.
                assert!(d.task.is_finished());
                assert!(d.shared.steps().contains(&Step::ReaderDropped(0)));
                assert_eq!(Instant::now(), before);
                return;
            }
            assert_eq!(d.watched().await, [Watching::default()], "{way:?}");
            if let TimedOut = way {
                assert!(Instant::now() - before < RELEASE_WAIT + secs(1));
            } else {
                assert!(Instant::now() - before < secs(1), "{way:?}");
            }
            d.assert_idle_after(&[]);
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_run_is_read_for_its_stop_hotkey_until_it_is_told_to_stop() {
    local(async {
        let d = registered_daemon(config(Mode::HoldButton), |_| {}).await;
        d.hotkey(reported()).await;
        d.start_by_hotkey().await;
        let running = stopping(&[Action::ToggleClicking]);
        assert_eq!(d.watched().await, [WAIT, running]);
        d.press_hotkey().await;
        assert_eq!(d.watched().await, [Watching::default()]);
        assert_eq!(d.activity(), Activity::Idle);
        // A tray start is read for its stop hotkey too.
        d.send(Message::Start).await;
        sleep(secs(5)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Clicking);
        let running = stopping(&[Action::ToggleClicking]);
        assert_eq!(d.watched().await, [running]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_hotkey_start_into_a_window_running_as_administrator_is_refused() {
    const REFUSED: &str =
        "Not started: Auto Clicker can't click into a program running as administrator";
    local(async {
        let d = registered_daemon(config(Mode::HoldButton), |r| r.elevated_front = true).await;
        d.hotkey(reported()).await;
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(d.notice(), REFUSED);
        assert_eq!(d.watched().await, []);
        // The tray's start is not asked: the taskbar is in front then.
        d.send(Message::Start).await;
        sleep(secs(5)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Clicking);
    })
    .await;
    // The window in front changes while the modifiers are still down.
    local(async {
        let d = registered_daemon(config(Mode::HoldButton), |_| {}).await;
        d.hotkey(reported()).await;
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
        d.shared.with(|r| r.elevated_front = true);
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        d.assert_idle_after(&[]);
        assert_eq!(d.notice(), REFUSED);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_hotkey_windows_refused_is_a_problem_and_cancels_no_start() {
    local(async {
        let d = registered_daemon(config(Mode::HoldButton), |_| {}).await;
        d.hotkey(reported()).await;
        d.press_hotkey().await;
        let taken = Unregistered {
            hotkey: Bindings::default().toggle_recording,
            why: NotRegistered::Taken,
        };
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Refused(vec![taken])))
            .await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
        assert_eq!(
            d.problems(),
            ["hotkey Ctrl+Shift+F7 does not work: another program uses it"]
        );
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        assert_eq!(d.activity(), Activity::Clicking);
    })
    .await;
}
