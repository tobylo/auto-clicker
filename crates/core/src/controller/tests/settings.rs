//! The settings window: saves are checked, written and taken into use by
//! the controller, which shows the window what it shows.

use std::fs;
use std::path::PathBuf;

use super::*;
use crate::config::{CountDraft, Draft, Field, IntervalDraft, ModeName, RouteName, Stale};
use crate::input::Button;
use crate::window::{Answer, Ask, Confirm, Outcome, Request, WindowView};

/// A settings window as the controller sees it: the other ends of a link.
struct Window {
    requests: mpsc::Sender<Request>,
    view: watch::Receiver<Option<WindowView>>,
    next_id: u64,
}

impl Window {
    fn open(opener: &WindowOpener) -> Window {
        let (requests, receiver) = mpsc::channel(4);
        let (sender, view) = watch::channel(None);
        opener
            .send(WindowLink {
                requests: receiver,
                view: sender,
            })
            .unwrap();
        Window {
            requests,
            view,
            next_id: 0,
        }
    }

    fn view(&self) -> WindowView {
        self.view.borrow().clone().expect("a view")
    }

    /// Asks `ask` and returns the answer the view shows for it.
    async fn ask(&mut self, d: &Daemon, ask: Ask) -> Outcome {
        self.next_id += 1;
        let id = self.next_id;
        self.requests.send(Request { id, ask }).await.unwrap();
        d.settle().await;
        let Some(Answer {
            id: answered,
            outcome,
        }) = self.view().answer
        else {
            panic!("no answer");
        };
        assert_eq!(answered, id);
        outcome
    }

    async fn save(&mut self, d: &Daemon, settings: Draft) -> Outcome {
        self.ask(d, save(settings, Agreed::default())).await
    }

    /// Whether the controller ended this window's session.
    fn ended(&self) -> bool {
        self.view.has_changed().is_err()
    }
}

/// What the user agreed to in a save.
#[derive(Default)]
struct Agreed {
    replace: bool,
    without_modifier: bool,
}

fn save(settings: Draft, agreed: Agreed) -> Ask {
    Ask::Save {
        settings,
        replace: agreed.replace,
        without_modifier: agreed.without_modifier,
    }
}

const REPLACE: Agreed = Agreed {
    replace: true,
    without_modifier: false,
};

/// A controller with a config file and a settings window open.
struct Setup {
    d: Daemon,
    window: Window,
    opener: WindowOpener,
    path: PathBuf,
    _dir: TempDir,
}

/// A daemon that read `text` as its config file (none when `None`), with a
/// window open on it.
async fn setup_with(text: Option<&str>, shared: Shared) -> Setup {
    let dir = TempDir::new();
    let path = dir.path().join("config.toml");
    if let Some(text) = text {
        fs::write(&path, text).unwrap();
    }
    let loaded = Config::load(&path);
    let mut opener = None;
    let d = daemon_prepared(loaded.config, shared, true, |controller| {
        controller.keep_config_file(loaded.file);
        opener = Some(controller.window_opener());
    })
    .await;
    let opener = opener.unwrap();
    let window = Window::open(&opener);
    d.settle().await;
    Setup {
        d,
        window,
        opener,
        path,
        _dir: dir,
    }
}

async fn setup() -> Setup {
    setup_with(None, Shared::default()).await
}

fn draft(change: impl FnOnce(&mut Draft)) -> Draft {
    let mut draft = Draft::from(&Config::default());
    change(&mut draft);
    draft
}

fn hold_right(d: &mut Draft) {
    d.clicker.mode = ModeName::HoldButton;
    d.clicker.button = crate::config::ButtonName::Right;
}

fn on_the_file(path: &PathBuf) -> Option<Config> {
    Config::parse(&fs::read_to_string(path).ok()?).ok()
}

fn refused_at(outcome: Outcome) -> Field {
    match outcome {
        Outcome::Refused(refusal) => refusal.field,
        other => panic!("not refused: {other:?}"),
    }
}

/// Bindings away from the defaults, F9 to F11.
fn moved(d: &mut Draft) {
    d.hotkeys.toggle_clicking = "Ctrl+F9".to_owned();
    d.hotkeys.toggle_recording = "Ctrl+F10".to_owned();
    d.hotkeys.toggle_playback = "Ctrl+F11".to_owned();
}

fn moved_bindings() -> Bindings {
    draft(moved).to_config().unwrap().hotkeys
}

// ---------------------------------------------------------------------------
// The view

#[tokio::test(start_paused = true)]
async fn an_opened_window_is_shown_the_settings_and_what_goes_on() {
    local(async {
        let s = setup().await;
        let view = s.window.view();
        assert_eq!(view.settings, Draft::from(&Config::default()));
        assert_eq!(view.running, Draft::from(&Config::default()).input);
        assert_eq!(view.activity, Activity::Idle);
        assert_eq!(view.hotkeys.toggle_clicking, s.d.view().hotkey);
        assert_eq!(view.no_recording, s.d.view().no_recording);
        assert_eq!(view.answer, None);
        s.d.start_by_hotkey().await;
        assert_eq!(s.window.view().activity, Activity::Clicking);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_new_window_ends_the_session_of_the_one_before() {
    local(async {
        let s = setup().await;
        let second = Window::open(&s.opener);
        s.d.settle().await;
        assert!(s.window.ended());
        assert!(!second.ended());
        assert_eq!(second.view().activity, Activity::Idle);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_window_that_closes_its_side_is_let_go_and_quit_lets_go_of_any() {
    local(async {
        let s = setup().await;
        let Window { requests, view, .. } = s.window;
        drop(requests);
        s.d.settle().await;
        assert!(view.has_changed().is_err());
        let other = Window::open(&s.opener);
        s.d.settle().await;
        s.d.send(Message::Quit).await;
        assert!(other.ended());
    })
    .await;
}

// ---------------------------------------------------------------------------
// Saving

#[tokio::test(start_paused = true)]
async fn a_save_is_written_shown_and_used_from_the_next_start() {
    local(async {
        let mut s = setup().await;
        let wanted = draft(hold_right);
        assert_eq!(s.window.save(&s.d, wanted.clone()).await, Outcome::Saved);
        assert_eq!(on_the_file(&s.path), wanted.to_config().ok());
        assert_eq!(s.window.view().settings, wanted);
        assert_eq!(s.d.view().settings.button, Button::Right);
        s.d.start_by_hotkey().await;
        assert_eq!(
            s.d.shared.calls(),
            [Call::Button(Button::Right, PressState::Down)]
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_save_waits_until_nothing_goes_on() {
    local(async {
        let shared = Shared::default();
        let mut s = setup_with(
            Some("version = 1\n[clicker]\nmode = \"hold_button\"\n"),
            shared,
        )
        .await;
        s.d.start_by_hotkey().await;
        assert_eq!(s.window.save(&s.d, draft(hold_right)).await, Outcome::Busy);
        s.d.send(Message::Stop).await;
        s.d.send(Message::Start).await;
        assert!(matches!(s.d.activity(), Activity::StartingIn(_)));
        assert_eq!(s.window.save(&s.d, draft(hold_right)).await, Outcome::Busy);
        s.d.send(Message::Stop).await;
        s.d.send(Message::Record).await;
        assert!(s.d.activity().recording());
        assert_eq!(s.window.save(&s.d, draft(hold_right)).await, Outcome::Busy);
        s.d.send(Message::StopRecording).await;
        assert_eq!(
            fs::read_to_string(&s.path).unwrap(),
            "version = 1\n[clicker]\nmode = \"hold_button\"\n"
        );
        assert_eq!(s.d.shared.calls(), [LEFT_DOWN, LEFT_UP]);
        assert_eq!(s.window.save(&s.d, draft(hold_right)).await, Outcome::Saved);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_invalid_value_is_refused_and_nothing_is_written() {
    local(async {
        let mut s = setup().await;
        let too_fast = draft(|d| d.clicker.interval = IntervalDraft::Ms(9));
        assert_eq!(
            refused_at(s.window.save(&s.d, too_fast).await),
            Field::Interval
        );
        let twice = draft(|d| d.hotkeys.toggle_playback = "Ctrl+Shift+F6".to_owned());
        assert_eq!(
            refused_at(s.window.save(&s.d, twice).await),
            Field::Hotkey(Action::TogglePlayback)
        );
        let never = draft(|d| d.playback.repeat = CountDraft::Times(0));
        assert_eq!(
            refused_at(s.window.save(&s.d, never).await),
            Field::PlaybackRepeat
        );
        assert!(!s.path.exists());
        assert_eq!(s.window.view().settings, Draft::from(&Config::default()));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_refusal_quoting_a_long_value_still_reaches_the_window() {
    local(async {
        let mut s = setup().await;
        let long = draft(|d| d.hotkeys.toggle_clicking = "x".repeat(40_000));
        let refusal = match s.window.save(&s.d, long).await {
            Outcome::Refused(refusal) => refusal,
            other => panic!("{other:?}"),
        };
        assert_eq!(refusal.field, Field::Hotkey(Action::ToggleClicking));
        assert_eq!(refusal.reason.chars().count(), crate::window::MAX_TEXT + 1);
        assert!(crate::window::line(&s.window.view()).is_ok());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_file_changed_on_disk_is_replaced_only_once_the_user_agrees() {
    local(async {
        let mut s = setup_with(Some("version = 1\n"), Shared::default()).await;
        fs::write(&s.path, "version = 1\ntray_start_delay = 1\n").unwrap();
        let wanted = draft(hold_right);
        assert_eq!(
            s.window.save(&s.d, wanted.clone()).await,
            Outcome::Confirm(Confirm::Replace(Stale::Changed))
        );
        assert_eq!(
            fs::read_to_string(&s.path).unwrap(),
            "version = 1\ntray_start_delay = 1\n"
        );
        assert_eq!(s.d.view().settings, ClickerSettings::default());
        let outcome = s.window.ask(&s.d, save(wanted.clone(), REPLACE)).await;
        assert_eq!(outcome, Outcome::Saved);
        assert_eq!(on_the_file(&s.path), wanted.to_config().ok());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_file_invalid_at_start_is_replaced_only_once_the_user_agrees_and_its_error_goes() {
    local(async {
        let mut s = setup_with(Some("version = 2\n"), Shared::default()).await;
        assert_eq!(
            s.window.save(&s.d, draft(hold_right)).await,
            Outcome::Confirm(Confirm::Replace(Stale::Unusable))
        );
        assert_eq!(fs::read_to_string(&s.path).unwrap(), "version = 2\n");
        let outcome = s.window.ask(&s.d, save(draft(hold_right), REPLACE)).await;
        assert_eq!(outcome, Outcome::Saved);
        assert!(s.window.view().problems.is_empty());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn without_a_config_file_a_save_fails_and_says_why() {
    local(async {
        let mut opener = None;
        let d = daemon_prepared(Config::default(), Shared::default(), true, |controller| {
            opener = Some(controller.window_opener());
        })
        .await;
        let mut window = Window::open(&opener.unwrap());
        d.settle().await;
        match window.save(&d, draft(hold_right)).await {
            Outcome::Failed(why) => assert!(why.contains("no config directory"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(d.view().settings, ClickerSettings::default());
    })
    .await;
}

// ---------------------------------------------------------------------------
// Hotkeys

#[tokio::test(start_paused = true)]
async fn a_hotkey_without_a_modifier_needs_the_users_word_once() {
    local(async {
        let mut s = setup().await;
        let plain = draft(|d| d.hotkeys.toggle_recording = "F13".to_owned());
        assert_eq!(
            s.window.save(&s.d, plain.clone()).await,
            Outcome::Confirm(Confirm::WithoutModifier(Action::ToggleRecording))
        );
        assert!(!s.path.exists());
        let outcome = s
            .window
            .ask(
                &s.d,
                save(
                    plain.clone(),
                    Agreed {
                        replace: false,
                        without_modifier: true,
                    },
                ),
            )
            .await;
        assert_eq!(outcome, Outcome::Saved);
        // Saved already: another change does not ask again.
        let mut more = plain;
        hold_right(&mut more);
        assert_eq!(s.window.save(&s.d, more).await, Outcome::Saved);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn saved_hotkeys_reach_the_reader_at_once_while_idle() {
    local(async {
        let mut s = setup().await;
        assert_eq!(s.window.save(&s.d, draft(moved)).await, Outcome::Saved);
        let started = s.d.shared.with(|r| r.bindings.clone());
        assert_eq!(started, [Bindings::default(), moved_bindings()]);
        assert_eq!(
            s.d.shared.steps(),
            [
                Step::ReaderStarted(Some(0)),
                Step::ReaderDropped(0),
                Step::ReaderStarted(Some(0)),
            ]
        );
        assert_eq!(
            s.d.view().hotkey,
            Hotkey::Bound(moved_bindings().toggle_clicking)
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_save_while_devices_open_again_is_refused() {
    local(async {
        let shared = Shared::default();
        let mut s = setup_with(
            Some("version = 1\n[clicker]\nmode = \"hold_button\"\n"),
            shared,
        )
        .await;
        s.d.start_by_hotkey().await;
        // A failed write loses the devices, which open again. The save is
        // handled before the open ends: no reader may start meanwhile.
        s.d.shared.log(0).fail_once(LEFT_UP);
        s.d.inbox.send(Message::Stop).unwrap();
        let held = draft(|d| {
            d.clicker.mode = ModeName::HoldButton;
            moved(d);
        });
        assert_eq!(s.window.save(&s.d, held.clone()).await, Outcome::Busy);
        assert_eq!(
            s.d.shared.steps(),
            [
                Step::ReaderStarted(Some(0)),
                Step::ReaderDropped(0),
                Step::Opened(Some(1)),
                Step::ReaderStarted(Some(1)),
            ]
        );
        assert_eq!(s.window.save(&s.d, held).await, Outcome::Saved);
        let started = s.d.shared.with(|r| r.bindings.clone());
        assert_eq!(started.last(), Some(&moved_bindings()));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_save_may_move_the_hotkeys_off_the_key_the_clicker_now_presses() {
    local(async {
        let mut s = setup().await;
        let on_f6 = draft(|d| {
            moved(d);
            d.clicker.mode = ModeName::KeyRepeat;
            d.clicker.key = 64;
        });
        assert_eq!(s.window.save(&s.d, on_f6).await, Outcome::Saved);
        let started = s.d.shared.with(|r| r.bindings.clone());
        assert_eq!(started.last(), Some(&moved_bindings()));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn moving_the_hotkeys_to_the_desktop_still_keeps_them_off_the_clicker_key() {
    local(async {
        let mut s = setup().await;
        let to_portal_on_f6 = draft(|d| {
            d.input.hotkeys = RouteName::Portal;
            d.clicker.mode = ModeName::KeyRepeat;
            d.clicker.key = 64;
        });
        assert!(to_portal_on_f6.to_config().is_ok());
        assert_eq!(
            refused_at(s.window.save(&s.d, to_portal_on_f6).await),
            Field::Hotkey(Action::ToggleClicking)
        );
        assert!(!s.path.exists());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn where_the_desktop_delivers_hotkeys_they_stay_and_the_rest_saves() {
    local(async {
        let shared = Shared::default();
        shared.with(|r| r.desktop_reader = true);
        let mut s = setup_with(Some("version = 1\n[input]\nhotkeys = \"portal\"\n"), shared).await;
        assert_eq!(
            s.window.view().no_recording,
            Some(NoRecording::DesktopHotkeys)
        );
        let portal = |d: &mut Draft| d.input.hotkeys = RouteName::Portal;
        let rebound = draft(|d| {
            portal(d);
            d.hotkeys.toggle_playback = "Ctrl+F11".to_owned();
        });
        let refusal = match s.window.save(&s.d, rebound).await {
            Outcome::Refused(refusal) => refusal,
            other => panic!("{other:?}"),
        };
        assert_eq!(refusal.field, Field::Hotkey(Action::TogglePlayback));
        assert!(
            refusal.reason.contains("System Settings"),
            "{}",
            refusal.reason
        );
        let other = draft(|d| {
            portal(d);
            hold_right(d);
        });
        assert_eq!(s.window.save(&s.d, other).await, Outcome::Saved);
        assert_eq!(s.d.shared.with(|r| r.bindings.len()), 1);
    })
    .await;
}
