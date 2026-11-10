//! The shortcut reader against scripted sessions and signals, on tokio's
//! paused clock: no portal is needed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::pending;
use std::rc::Rc;
use std::time::Duration;

use auto_clicker_core::hotkeys::{
    Action, Binding, DESKTOP_RELEASE_PAUSE, HotkeyEvent, HotkeyStatus, Trigger, Triggers,
    Unavailable,
};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::yield_now;
use tokio::time::{Instant, advance};
use zbus::zvariant::{ObjectPath, Value};

use super::portal::{failed, parse, preferred_trigger};
use super::reactor::{Listed, Signal, bound_triggers, serve};
use super::*;

const CLICKING: &str = "toggle_clicking";
const RECORDING: &str = "toggle_recording";

fn listed(pairs: &[(&str, &str)]) -> Vec<Listed> {
    pairs
        .iter()
        .map(|(id, trigger)| ((*id).to_owned(), (*trigger).to_owned()))
        .collect()
}

fn defaults() -> Vec<Listed> {
    listed(&[
        (CLICKING, "Ctrl+Shift+F6"),
        (RECORDING, "Ctrl+Shift+F7"),
        ("toggle_playback", "Ctrl+Shift+F8"),
    ])
}

fn fired(action: Action) -> HotkeyEvent {
    HotkeyEvent::Fired(action)
}

fn lost(why: &str) -> HotkeyEvent {
    HotkeyEvent::Status(HotkeyStatus::Unavailable(Unavailable::Desktop(
        why.to_owned(),
    )))
}

/// Lets the reader handle what was sent, without moving the clock.
async fn settle() {
    for _ in 0..20 {
        yield_now().await;
    }
}

/// One served session: its signals go in, its events and its end come out.
struct Served {
    signals: mpsc::UnboundedSender<Signal>,
    events: Rc<RefCell<Vec<HotkeyEvent>>>,
    end: tokio::task::JoinHandle<String>,
}

impl Served {
    /// Serves a session bound with the default triggers; the two events
    /// every session opens with are checked and left out.
    async fn start() -> Served {
        let (signals, received) = mpsc::unbounded_channel();
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let end = tokio::task::spawn_local(async move {
            let signals = futures_util::stream::unfold(received, |mut received| async {
                received.recv().await.map(|signal| (signal, received))
            });
            let mut send = |event| sink.borrow_mut().push(event);
            serve(bound_triggers(&defaults()), signals, &mut send).await
        });
        settle().await;
        let served = Served {
            signals,
            events,
            end,
        };
        assert_eq!(
            served.take(),
            [
                HotkeyEvent::Triggers(bound_triggers(&defaults())),
                HotkeyEvent::Status(HotkeyStatus::Available),
            ]
        );
        served
    }

    async fn hear(&self, signal: Signal) {
        self.signals.send(signal).unwrap();
        settle().await;
    }

    async fn press(&self, id: &str) {
        self.hear(Signal::Activated(id.to_owned())).await;
    }

    async fn let_go(&self, id: &str) {
        self.hear(Signal::Deactivated(id.to_owned())).await;
    }

    /// The events since the last call.
    fn take(&self) -> Vec<HotkeyEvent> {
        self.events.borrow_mut().drain(..).collect()
    }
}

async fn local(body: impl Future<Output = ()>) {
    tokio::task::LocalSet::new().run_until(body).await;
}

// One session

#[test]
fn a_bind_answer_gives_each_action_its_trigger_and_no_trigger_to_the_rest() {
    let triggers = bound_triggers(&listed(&[
        (RECORDING, "Meta+F7"),
        ("something_else", "Ctrl+F1"),
    ]));
    let mut expected = Triggers::unknown();
    expected.set(Action::ToggleClicking, Trigger::reported(""));
    expected.set(Action::ToggleRecording, Trigger::reported("Meta+F7"));
    expected.set(Action::TogglePlayback, Trigger::reported(""));
    assert_eq!(triggers, expected);
}

#[tokio::test(start_paused = true)]
async fn a_press_fires_at_once_and_the_release_comes_a_pause_after_letting_go() {
    local(async {
        let s = Served::start().await;
        let start = Instant::now();
        s.press(CLICKING).await;
        assert_eq!(s.take(), [fired(Action::ToggleClicking)]);
        assert_eq!(start.elapsed(), Duration::ZERO);
        // Held: no release, however long.
        advance(Duration::from_secs(20)).await;
        settle().await;
        assert_eq!(s.take(), []);
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE - Duration::from_millis(1)).await;
        settle().await;
        assert_eq!(s.take(), [], "released before the pause was over");
        advance(Duration::from_millis(1)).await;
        settle().await;
        assert_eq!(s.take(), [HotkeyEvent::ModifiersReleased]);
        // Once.
        advance(Duration::from_secs(60)).await;
        settle().await;
        assert_eq!(s.take(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn every_action_fires_by_its_id_and_unknown_ids_do_nothing() {
    local(async {
        let s = Served::start().await;
        for (id, action) in [
            (CLICKING, Action::ToggleClicking),
            (RECORDING, Action::ToggleRecording),
            ("toggle_playback", Action::TogglePlayback),
        ] {
            s.press(id).await;
            s.let_go(id).await;
            assert_eq!(s.take(), [fired(action)], "{id}");
        }
        s.press("clicking").await;
        s.press("").await;
        assert_eq!(s.take(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_repeated_activation_while_held_fires_once() {
    local(async {
        let s = Served::start().await;
        for _ in 0..5 {
            s.press(CLICKING).await;
        }
        assert_eq!(s.take(), [fired(Action::ToggleClicking)]);
        // Let go and pressed again: that is a new press.
        s.let_go(CLICKING).await;
        s.press(CLICKING).await;
        assert_eq!(s.take(), [fired(Action::ToggleClicking)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_lost_release_blocks_at_most_the_next_press() {
    local(async {
        let s = Served::start().await;
        s.press(CLICKING).await;
        // Its Deactivated never arrives. Another shortcut's release, or a
        // change of triggers, clears it.
        s.let_go(RECORDING).await;
        s.press(CLICKING).await;
        assert_eq!(s.take().len(), 2);
        s.hear(Signal::Changed(Vec::new())).await;
        s.press(CLICKING).await;
        let events = s.take();
        assert_eq!(events.last(), Some(&fired(Action::ToggleClicking)));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_second_press_during_the_pause_shares_one_later_release() {
    local(async {
        let s = Served::start().await;
        s.press(CLICKING).await;
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE / 2).await;
        // Pressed again before the release was reported: a cancel.
        s.press(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE).await;
        settle().await;
        assert_eq!(
            s.take(),
            [fired(Action::ToggleClicking), fired(Action::ToggleClicking)],
            "the first press's release was reported while the second was held"
        );
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE).await;
        settle().await;
        assert_eq!(s.take(), [HotkeyEvent::ModifiersReleased]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_release_without_a_press_reports_nothing() {
    local(async {
        let s = Served::start().await;
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE * 3).await;
        settle().await;
        assert_eq!(s.take(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn changed_triggers_are_reported_whole_with_the_change_applied() {
    local(async {
        let s = Served::start().await;
        s.hear(Signal::Changed(listed(&[(RECORDING, "Meta+F11")])))
            .await;
        let mut expected = bound_triggers(&defaults());
        expected.set(Action::ToggleRecording, Trigger::reported("Meta+F11"));
        assert_eq!(s.take(), [HotkeyEvent::Triggers(expected.clone())]);
        // A trigger removed in the desktop's settings.
        s.hear(Signal::Changed(listed(&[(CLICKING, "")]))).await;
        expected.set(Action::ToggleClicking, Trigger::reported(""));
        assert_eq!(s.take(), [HotkeyEvent::Triggers(expected)]);
        // The session goes on.
        s.press(RECORDING).await;
        assert_eq!(s.take(), [fired(Action::ToggleRecording)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_session_ends_when_closed_unreadable_or_cut_off_and_says_which() {
    local(async {
        let cases: [(Option<Signal>, &str); 3] = [
            (Some(Signal::Closed), "closed the shortcuts session"),
            (
                Some(Signal::Malformed("Activated".to_owned())),
                "a Activated signal the program cannot read",
            ),
            (None, "the session bus connection ended"),
        ];
        for (signal, why) in cases {
            let s = Served::start().await;
            s.press(CLICKING).await;
            let Served { signals, end, .. } = s;
            match signal {
                Some(signal) => signals.send(signal).unwrap(),
                None => drop(signals),
            }
            let said = tokio::time::timeout(Duration::from_millis(1), end)
                .await
                .expect("the session did not end at once")
                .unwrap();
            assert!(said.contains(why), "{said}");
        }
    })
    .await;
}

// Sessions one after another

/// A scripted desktop: each session notes that it started and says hotkeys
/// work, then ends with its scripted reason, or never.
struct Script {
    /// What each session does, in order; sessions past the end never end.
    sessions: Vec<Option<&'static str>>,
    started: Rc<RefCell<usize>>,
}

impl Desktop for Script {
    async fn session(&mut self, send: &mut dyn FnMut(HotkeyEvent)) -> String {
        let n = {
            let mut started = self.started.borrow_mut();
            *started += 1;
            *started - 1
        };
        send(HotkeyEvent::Status(HotkeyStatus::Available));
        match self.sessions.get(n).copied().flatten() {
            Some(why) => why.to_owned(),
            None => pending().await,
        }
    }
}

/// A running reader: owner changes go in, events come out.
struct Reader {
    owners: Option<mpsc::UnboundedSender<bool>>,
    events: Rc<RefCell<Vec<HotkeyEvent>>>,
    started: Rc<RefCell<usize>>,
    task: tokio::task::JoinHandle<()>,
}

impl Reader {
    async fn start(sessions: Vec<Option<&'static str>>) -> Reader {
        let (owners, changes) = mpsc::unbounded_channel();
        let events = Rc::new(RefCell::new(Vec::new()));
        let started = Rc::new(RefCell::new(0));
        let mut script = Script {
            sessions,
            started: started.clone(),
        };
        let sink = events.clone();
        let task = tokio::task::spawn_local(async move {
            let changes = futures_util::stream::unfold(changes, |mut changes| async {
                changes.recv().await.map(|change| (change, changes))
            });
            run(&mut script, changes.boxed_local(), |event| {
                sink.borrow_mut().push(event);
            })
            .await;
        });
        settle().await;
        Reader {
            owners: Some(owners),
            events,
            started,
            task,
        }
    }

    async fn owner(&self, present: bool) {
        self.owners.as_ref().unwrap().send(present).unwrap();
        settle().await;
    }

    fn take(&self) -> Vec<HotkeyEvent> {
        self.events.borrow_mut().drain(..).collect()
    }

    fn sessions(&self) -> usize {
        *self.started.borrow()
    }
}

const AVAILABLE: HotkeyEvent = HotkeyEvent::Status(HotkeyStatus::Available);

#[tokio::test(start_paused = true)]
async fn a_portal_that_stops_is_reported_and_bound_again_when_it_returns() {
    local(async {
        let r = Reader::start(Vec::new()).await;
        assert_eq!(r.take(), [AVAILABLE]);
        r.owner(false).await;
        assert_eq!(r.take(), [lost("the desktop's portal service stopped")]);
        // No retry while it is away, however long.
        advance(Duration::from_secs(3600)).await;
        settle().await;
        assert_eq!((r.sessions(), r.take()), (1, Vec::new()));
        r.owner(true).await;
        assert_eq!((r.sessions(), r.take()), (2, vec![AVAILABLE]));
        assert!(!r.task.is_finished());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_portal_replaced_in_one_step_is_bound_again_at_once() {
    local(async {
        let r = Reader::start(Vec::new()).await;
        r.take();
        r.owner(true).await;
        assert_eq!(r.sessions(), 2);
        assert_eq!(
            r.take(),
            [lost("the desktop's portal service restarted"), AVAILABLE]
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_session_that_fails_is_not_retried_until_the_portal_returns() {
    local(async {
        let r = Reader::start(vec![Some("not allowed"), Some("still not")]).await;
        assert_eq!(r.take(), [AVAILABLE, lost("not allowed")]);
        advance(Duration::from_secs(3600)).await;
        settle().await;
        // The portal leaving changes nothing; its return is a new try.
        r.owner(false).await;
        assert_eq!((r.sessions(), r.take()), (1, Vec::new()));
        r.owner(true).await;
        assert_eq!(r.sessions(), 2);
        assert_eq!(r.take(), [AVAILABLE, lost("still not")]);
        r.owner(true).await;
        assert_eq!((r.sessions(), r.take()), (3, vec![AVAILABLE]));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_reader_ends_when_the_bus_can_no_longer_be_watched() {
    local(async {
        // While a session runs, and while waiting for the portal.
        for sessions in [Vec::new(), vec![Some("failed")]] {
            let mut r = Reader::start(sessions).await;
            r.take();
            r.owners = None;
            settle().await;
            assert!(r.task.is_finished());
            let events = r.take();
            assert!(
                events.len() <= 1,
                "one last report at most, then nothing: {events:?}"
            );
        }
        let mut r = Reader::start(Vec::new()).await;
        r.take();
        r.owners = None;
        settle().await;
        let events = r.take();
        assert!(
            matches!(
                events.as_slice(),
                [HotkeyEvent::Status(HotkeyStatus::Unavailable(Unavailable::Desktop(why)))]
                    if why.contains("can no longer be watched")
            ),
            "{events:?}"
        );
    })
    .await;
}

// The portal's side

#[test]
fn a_binding_on_a_function_key_is_asked_for_as_the_specification_spells_it() {
    let cases = [
        ("Ctrl+Shift+F6", Some("CTRL+SHIFT+F6")),
        ("F24", Some("F24")),
        ("Meta+Alt+F1", Some("ALT+LOGO+F1")),
        ("Ctrl+Space", None),
        ("Ctrl+F", None),
        ("Ctrl+W", None),
    ];
    for (binding, trigger) in cases {
        let binding = Binding::parse(binding).unwrap();
        assert_eq!(preferred_trigger(binding).as_deref(), trigger, "{binding}");
    }
}

fn signal<B>(member: &str, body: &B) -> zbus::Message
where
    B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
{
    zbus::Message::signal(
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.GlobalShortcuts",
        member,
    )
    .unwrap()
    .build(body)
    .unwrap()
}

fn session_path() -> ObjectPath<'static> {
    ObjectPath::try_from("/org/freedesktop/portal/desktop/session/1_2/t").unwrap()
}

#[test]
fn the_portals_signals_parse_into_what_they_say() {
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    let pressed = signal("Activated", &(session_path(), CLICKING, 0u64, &options));
    assert_eq!(parse(&pressed), Signal::Activated(CLICKING.to_owned()));
    let let_go = signal("Deactivated", &(session_path(), RECORDING, 0u64, &options));
    assert_eq!(parse(&let_go), Signal::Deactivated(RECORDING.to_owned()));
    let info: HashMap<&str, Value<'_>> = HashMap::from([
        ("description", Value::from("Start or stop clicking")),
        ("trigger_description", Value::from("Ctrl+Shift+F6")),
    ]);
    let changed = signal(
        "ShortcutsChanged",
        &(session_path(), vec![(CLICKING, info)]),
    );
    assert_eq!(
        parse(&changed),
        Signal::Changed(listed(&[(CLICKING, "Ctrl+Shift+F6")]))
    );
}

#[test]
fn a_signal_that_does_not_parse_is_malformed_not_ignored() {
    // An activation without its shortcut id, and a signal nobody defined.
    let short = signal("Activated", &(session_path(),));
    assert_eq!(parse(&short), Signal::Malformed("Activated".to_owned()));
    let wrong = signal("ShortcutsChanged", &(session_path(), "not a list"));
    assert_eq!(
        parse(&wrong),
        Signal::Malformed("ShortcutsChanged".to_owned())
    );
    let unknown = signal("Reconfigured", &(session_path(),));
    assert_eq!(
        parse(&unknown),
        Signal::Malformed("Reconfigured".to_owned())
    );
}

#[test]
fn failed_calls_say_what_the_user_can_do() {
    use ashpd::desktop::ResponseError;
    let cancelled = failed(
        "BindShortcuts",
        &ashpd::Error::Response(ResponseError::Cancelled),
    );
    assert!(
        cancelled.contains("not allowed in the desktop's dialog"),
        "{cancelled}"
    );
    let no_app_id = failed(
        "CreateSession",
        &ashpd::Error::Portal(ashpd::PortalError::NotAllowed(
            "An app id is required".to_owned(),
        )),
    );
    assert!(
        no_app_id.contains("An app id is required") && no_app_id.contains(".desktop"),
        "{no_app_id}"
    );
    let other = failed("CreateSession", &ashpd::Error::NoResponse);
    assert!(other.starts_with("CreateSession: "), "{other}");
}
