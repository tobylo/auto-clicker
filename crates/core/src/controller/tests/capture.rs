//! Captures and the desktop's shortcut settings, asked for by the settings
//! window: hotkeys come back on every way a capture ends, at once, and
//! every wait ends by its deadline.
//!
//! "At once" means before any virtual time passed: `settle` only yields.

use super::settings::{Setup, Window, draft, setup, setup_with};
use super::*;
use crate::hotkeys::{Captured, Trigger, Triggers};
use crate::window::{Ask, CAPTURE_TIME, CaptureEnd, Outcome};

fn ctrl_f9() -> Binding {
    Binding::parse("Ctrl+F9").unwrap()
}

/// What each reader started so far was started to capture.
fn captures(d: &Daemon) -> Vec<Option<Capture>> {
    d.shared.with(|r| r.captures.clone())
}

/// Whether the newest reader reads the hotkeys, with the config's bindings.
fn hotkeys_back(d: &Daemon) -> bool {
    let last = d
        .shared
        .with(|r| (r.captures.last().copied(), r.bindings.last().copied()));
    last == (Some(None), Some(Bindings::default()))
}

/// Starts a hotkey capture and checks the reader was started for it.
async fn capturing(s: &mut Setup) -> u64 {
    let id = s.window.send(&s.d, Ask::Capture(Capture::Hotkey)).await;
    assert_eq!(
        s.window.answer_to(id),
        None,
        "a capture answers when it ends"
    );
    assert_eq!(captures(&s.d).last(), Some(&Some(Capture::Hotkey)));
    id
}

fn ended(why: CaptureEnd) -> Option<Outcome> {
    Some(Outcome::CaptureEnded(why))
}

#[tokio::test(start_paused = true)]
async fn a_captured_hotkey_is_the_answer_and_the_hotkeys_come_back() {
    local(async {
        let mut s = setup().await;
        let id = capturing(&mut s).await;
        s.d.hotkey(HotkeyEvent::Captured(Captured::Hotkey(ctrl_f9())))
            .await;
        assert_eq!(
            s.window.answer_to(id),
            Some(Outcome::CapturedHotkey(ctrl_f9()))
        );
        assert!(hotkeys_back(&s.d));
        assert_eq!(
            s.d.shared.steps(),
            [
                Step::ReaderStarted(Some(0)),
                Step::ReaderDropped(0),
                Step::ReaderStarted(Some(0)),
                Step::ReaderDropped(1),
                Step::ReaderStarted(Some(0)),
            ]
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_captured_key_is_its_code() {
    local(async {
        let mut s = setup().await;
        let id = s.window.send(&s.d, Ask::Capture(Capture::Key)).await;
        let space = Key::from_name("space").unwrap();
        s.d.hotkey(HotkeyEvent::Captured(Captured::Key(space)))
            .await;
        assert_eq!(s.window.answer_to(id), Some(Outcome::CapturedKey(57)));
        assert!(hotkeys_back(&s.d));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn esc_and_a_key_without_a_name_end_it_without_a_key() {
    local(async {
        let mut s = setup().await;
        for (captured, why) in [
            (Captured::Cancelled, CaptureEnd::Cancelled),
            (Captured::Unnamed, CaptureEnd::Unnamed),
        ] {
            let id = capturing(&mut s).await;
            s.d.hotkey(HotkeyEvent::Captured(captured)).await;
            assert_eq!(s.window.answer_to(id), ended(why));
            assert!(hotkeys_back(&s.d));
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_capture_ends_at_its_deadline_and_not_before() {
    local(async {
        let mut s = setup().await;
        let id = capturing(&mut s).await;
        sleep(CAPTURE_TIME - Duration::from_millis(1)).await;
        s.d.settle().await;
        assert_eq!(s.window.answer_to(id), None);
        sleep(Duration::from_millis(1)).await;
        s.d.settle().await;
        assert_eq!(s.window.answer_to(id), ended(CaptureEnd::TimedOut));
        assert!(hotkeys_back(&s.d));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_window_closing_or_a_new_one_ends_it_at_once() {
    local(async {
        let mut s = setup().await;
        capturing(&mut s).await;
        let Window { requests, view, .. } = s.window;
        drop((requests, view));
        s.d.settle().await;
        assert!(hotkeys_back(&s.d));
        let mut first = Window::open(&s.opener);
        s.d.settle().await;
        let id = first.send(&s.d, Ask::Capture(Capture::Hotkey)).await;
        assert_eq!(captures(&s.d).last(), Some(&Some(Capture::Hotkey)));
        let second = Window::open(&s.opener);
        s.d.settle().await;
        assert!(hotkeys_back(&s.d));
        assert!(first.ended() && !second.ended(), "{id}");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn every_message_but_the_menu_and_the_layout_ends_it_at_once() {
    local(async {
        let mut s = setup().await;
        let quiet = [Message::MenuOpened, Message::LayoutChanged];
        for message in quiet {
            capturing(&mut s).await;
            s.d.send(message).await;
            assert_eq!(captures(&s.d).last(), Some(&Some(Capture::Hotkey)));
            s.d.hotkey(HotkeyEvent::Captured(Captured::Cancelled)).await;
        }
        let ending = [
            Message::Stop,
            Message::StopRecording,
            Message::LockRequested,
            Message::Session(None),
            Message::Session(Some(Blocked::Locked)),
            Message::Sleep(1),
        ];
        for message in ending {
            let id = capturing(&mut s).await;
            s.d.send(message.clone()).await;
            let interrupted = ended(CaptureEnd::Interrupted);
            assert_eq!(s.window.answer_to(id), interrupted, "{message:?}");
            assert!(hotkeys_back(&s.d), "{message:?}");
            s.d.send(Message::Session(None)).await;
        }
        // Quit ends the controller, and with it the window's session.
        capturing(&mut s).await;
        s.d.send(Message::Quit).await;
        assert!(s.window.ended());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_start_from_the_tray_ends_it_first_so_the_reader_knows_the_devices() {
    local(async {
        let shared = Shared::default();
        let mut s = setup_with(Some("version = 1\ntray_start_delay = 0\n"), shared).await;
        let id = capturing(&mut s).await;
        s.d.send(Message::Start).await;
        assert_eq!(s.window.answer_to(id), ended(CaptureEnd::Interrupted));
        assert_eq!(s.d.activity(), Activity::Clicking);
        // The reader started for the hotkeys before the devices went into
        // the run, so it knows them.
        assert_eq!(
            s.d.shared.steps().last(),
            Some(&Step::ReaderStarted(Some(0)))
        );
        assert!(hotkeys_back(&s.d));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn keyboards_that_stop_working_end_it_and_say_why() {
    local(async {
        let mut s = setup().await;
        let id = capturing(&mut s).await;
        let gone = Unavailable::WatchFailed("gone".to_owned());
        s.d.hotkey(HotkeyEvent::Status(HotkeyStatus::Unavailable(gone.clone())))
            .await;
        assert_eq!(
            s.window.answer_to(id),
            Some(Outcome::Unavailable(gone.to_string()))
        );
        assert!(hotkeys_back(&s.d));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn nothing_else_is_asked_while_it_runs() {
    local(async {
        let mut s = setup().await;
        capturing(&mut s).await;
        assert_eq!(s.window.save(&s.d, draft(|_| {})).await, Outcome::Busy);
        let again = s.window.ask(&s.d, Ask::Capture(Capture::Key)).await;
        assert_eq!(again, Outcome::Busy);
        assert_eq!(captures(&s.d).last(), Some(&Some(Capture::Hotkey)));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn no_capture_starts_while_anything_goes_on() {
    local(async {
        let shared = Shared::default();
        let mut s = setup_with(
            Some("version = 1\n[clicker]\nmode = \"hold_button\"\n"),
            shared,
        )
        .await;
        s.d.start_by_hotkey().await;
        let asked = s.window.ask(&s.d, Ask::Capture(Capture::Hotkey)).await;
        assert_eq!(asked, Outcome::Busy);
        s.d.send(Message::Stop).await;
        s.d.send(Message::Record).await;
        let asked = s.window.ask(&s.d, Ask::Capture(Capture::Hotkey)).await;
        assert_eq!(asked, Outcome::Busy);
        assert!(captures(&s.d).iter().all(Option::is_none));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_capture_reader_finds_out_whether_a_keyboard_can_be_read() {
    local(async {
        let mut s = setup().await;
        // The hotkeys' reader found none with a hotkey key: a capture reads
        // more, so it starts, and ends as soon as its reader finds none.
        let none = Unavailable::NoKeyboard;
        let status = HotkeyEvent::Status(HotkeyStatus::Unavailable(none.clone()));
        s.d.hotkey(status.clone()).await;
        let id = capturing(&mut s).await;
        s.d.hotkey(status).await;
        assert_eq!(
            s.window.answer_to(id),
            Some(Outcome::Unavailable(none.to_string()))
        );
        assert!(hotkeys_back(&s.d));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn no_capture_starts_while_the_session_may_not_be_read() {
    local(async {
        let mut s = setup().await;
        s.d.send(Message::Session(Some(Blocked::Locked))).await;
        let asked = s.window.ask(&s.d, Ask::Capture(Capture::Hotkey)).await;
        assert_eq!(asked, Outcome::Unavailable(Blocked::Locked.to_string()));
        assert!(captures(&s.d).iter().all(Option::is_none));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_key_the_controller_gets_to_only_at_the_deadline_is_not_taken() {
    local(async {
        let mut s = setup().await;
        let id = capturing(&mut s).await;
        tokio::time::advance(CAPTURE_TIME).await;
        s.d.hotkey(HotkeyEvent::Captured(Captured::Hotkey(ctrl_f9())))
            .await;
        assert_eq!(s.window.answer_to(id), ended(CaptureEnd::TimedOut));
        assert!(hotkeys_back(&s.d));
    })
    .await;
}

// ---------------------------------------------------------------------------
// The desktop's shortcut settings

/// A daemon whose hotkeys come from the desktop, with a window open.
async fn desktop_setup() -> Setup {
    let shared = Shared::default();
    shared.with(|r| r.desktop_reader = true);
    let s = setup_with(Some("version = 1\n[input]\nhotkeys = \"portal\"\n"), shared).await;
    let mut triggers = Triggers::unknown();
    for action in Action::ALL {
        triggers.set(action, Trigger::reported(""));
    }
    s.d.hotkey(HotkeyEvent::Triggers(triggers)).await;
    s
}

/// The next command the newest reader got, if one is waiting.
async fn command(d: &Daemon) -> Option<ReaderCommand> {
    let mut commands = d.shared.with(|r| r.commands.pop())?;
    let next = timeout(Duration::from_millis(1), commands.next())
        .await
        .ok();
    d.shared.with(|r| r.commands.push(commands));
    next
}

/// The reader's answer to operation `operation`.
fn answered(operation: u64, result: Result<(), &str>) -> HotkeyEvent {
    HotkeyEvent::Configured {
        operation,
        result: result.map_err(str::to_owned),
    }
}

fn configure(token: Option<&str>) -> Ask {
    Ask::ConfigureShortcuts {
        activation_token: token.map(str::to_owned),
    }
}

#[tokio::test(start_paused = true)]
async fn the_desktops_settings_open_through_the_reader() {
    local(async {
        let mut s = desktop_setup().await;
        let id = s.window.send(&s.d, configure(Some("token-1"))).await;
        let asked = ReaderCommand::ConfigureShortcuts {
            operation: 1,
            activation_token: Some("token-1".to_owned()),
        };
        assert_eq!(command(&s.d).await, Some(asked));
        assert_eq!(s.window.answer_to(id), None);
        s.d.hotkey(answered(1, Ok(()))).await;
        assert_eq!(s.window.answer_to(id), Some(Outcome::Configured));
        let id = s.window.send(&s.d, configure(None)).await;
        s.d.hotkey(answered(2, Err("refused"))).await;
        assert_eq!(
            s.window.answer_to(id),
            Some(Outcome::Failed("refused".to_owned()))
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_reader_that_never_answers_is_given_up_on_and_one_thing_at_a_time() {
    local(async {
        let mut s = desktop_setup().await;
        let id = s.window.send(&s.d, configure(None)).await;
        assert_eq!(s.window.ask(&s.d, configure(None)).await, Outcome::Busy);
        sleep(Duration::from_secs(60)).await;
        s.d.settle().await;
        let failed = Some(Outcome::Failed(
            "the desktop did not open its settings".to_owned(),
        ));
        assert_eq!(s.window.answer_to(id), failed);
        // A late answer changes nothing.
        s.d.hotkey(answered(1, Ok(()))).await;
        assert_eq!(s.window.answer_to(id), failed);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_answer_for_a_window_since_gone_is_not_taken_for_the_next_one() {
    local(async {
        let s = desktop_setup().await;
        let mut first = s.window;
        first.send(&s.d, configure(None)).await;
        let mut second = Window::open(&s.opener);
        s.d.settle().await;
        let id = second.send(&s.d, configure(None)).await;
        s.d.hotkey(answered(1, Ok(()))).await;
        assert_eq!(second.answer_to(id), None);
        s.d.hotkey(answered(2, Err("refused"))).await;
        let refused = Some(Outcome::Failed("refused".to_owned()));
        assert_eq!(second.answer_to(id), refused);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_wait_for_the_desktops_settings_leaves_a_hotkey_release_wait_its_own_deadline() {
    local(async {
        let mut s = desktop_setup().await;
        s.window.send(&s.d, configure(None)).await;
        sleep(Duration::from_secs(40)).await;
        s.d.press_hotkey().await;
        assert_eq!(s.d.activity(), Activity::WaitingForRelease);
        sleep(Duration::from_secs(21)).await;
        s.d.settle().await;
        // The settings' deadline passed at 60 s; the release wait runs to 70 s.
        assert_eq!(s.d.activity(), Activity::WaitingForRelease);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_wait_for_the_desktops_settings_ends_during_a_run_too() {
    local(async {
        let mut s = desktop_setup().await;
        s.d.send(Message::Start).await;
        let id = s.window.send(&s.d, configure(None)).await;
        sleep(Duration::from_secs(60)).await;
        s.d.settle().await;
        assert!(s.d.activity().clicking());
        let failed = Some(Outcome::Failed(
            "the desktop did not open its settings".to_owned(),
        ));
        assert_eq!(s.window.answer_to(id), failed);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_countdowns_seconds_do_not_end_the_wait_for_the_desktops_settings() {
    local(async {
        let mut s = desktop_setup().await;
        s.d.send(Message::Start).await;
        let id = s.window.send(&s.d, configure(None)).await;
        sleep(Duration::from_secs(2)).await;
        s.d.settle().await;
        assert!(matches!(s.d.activity(), Activity::StartingIn(_)));
        assert_eq!(s.window.answer_to(id), None);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_token_that_is_no_token_is_refused() {
    local(async {
        let mut s = desktop_setup().await;
        let bad = s.window.ask(&s.d, configure(Some("two words"))).await;
        assert!(matches!(bad, Outcome::Failed(_)), "{bad:?}");
        assert_eq!(command(&s.d).await, None);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_routes_each_offer_only_their_own() {
    local(async {
        let mut kernel = setup().await;
        let asked = kernel.window.ask(&kernel.d, configure(None)).await;
        assert!(matches!(asked, Outcome::Unavailable(_)), "{asked:?}");
        let mut desktop = desktop_setup().await;
        let asked = desktop
            .window
            .ask(&desktop.d, Ask::Capture(Capture::Hotkey))
            .await;
        assert!(matches!(asked, Outcome::Unavailable(_)), "{asked:?}");
        assert!(captures(&desktop.d).iter().all(Option::is_none));
    })
    .await;
}
