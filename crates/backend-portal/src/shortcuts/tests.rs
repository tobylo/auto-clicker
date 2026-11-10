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
use auto_clicker_core::input::InputError;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::yield_now;
use tokio::time::{Instant, advance};
use zbus::zvariant::{ObjectPath, Value};

use super::portal::{
    MAX_LISTED, closes_a_session, connect_failure, failed, has_owner, parse, preferred_trigger,
    session_namespace, within,
};
use super::reactor::{Listed, Signal, bound_triggers, id_of, serve};
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
async fn every_activation_fires() {
    local(async {
        // The desktop sends one per press and releases a shortcut before
        // it activates another; nothing is held back on a guess.
        let s = Served::start().await;
        s.press(CLICKING).await;
        s.let_go(CLICKING).await;
        s.press(RECORDING).await;
        s.let_go(RECORDING).await;
        s.press(CLICKING).await;
        assert_eq!(
            s.take(),
            [
                fired(Action::ToggleClicking),
                fired(Action::ToggleRecording),
                fired(Action::ToggleClicking)
            ]
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_stray_release_after_the_reported_one_reports_nothing_more() {
    local(async {
        let s = Served::start().await;
        s.press(CLICKING).await;
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE).await;
        settle().await;
        assert_eq!(
            s.take(),
            [
                fired(Action::ToggleClicking),
                HotkeyEvent::ModifiersReleased
            ]
        );
        s.let_go(CLICKING).await;
        advance(DESKTOP_RELEASE_PAUSE * 2).await;
        settle().await;
        assert_eq!(s.take(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_press_that_arrives_with_a_due_release_comes_first() {
    local(async {
        let s = Served::start().await;
        s.press(CLICKING).await;
        s.let_go(CLICKING).await;
        // The second press is queued when the pause runs out: it is handled
        // before the release, which then belongs to it and waits for its
        // own letting go.
        s.signals
            .send(Signal::Activated(CLICKING.to_owned()))
            .unwrap();
        advance(DESKTOP_RELEASE_PAUSE).await;
        settle().await;
        assert_eq!(
            s.take(),
            [fired(Action::ToggleClicking), fired(Action::ToggleClicking)]
        );
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

/// What one scripted session does.
#[derive(Clone, Copy)]
enum Plan {
    /// Ends with this reason before it has bound the shortcuts.
    Fails(&'static str),
    /// Binds, says hotkeys work, then ends with this reason.
    BindsThenEnds(&'static str),
    /// Binds, says hotkeys work, and runs until dropped.
    Serves,
}

/// A scripted desktop: each session notes that it started and follows its
/// plan; sessions past the end of the script serve.
struct Script {
    sessions: Vec<Plan>,
    started: Rc<RefCell<usize>>,
}

impl Desktop for Script {
    async fn session(&mut self, send: &mut dyn FnMut(HotkeyEvent)) -> String {
        let n = {
            let mut started = self.started.borrow_mut();
            *started += 1;
            *started - 1
        };
        match self.sessions.get(n).copied().unwrap_or(Plan::Serves) {
            Plan::Fails(why) => why.to_owned(),
            Plan::BindsThenEnds(why) => {
                send(AVAILABLE);
                why.to_owned()
            }
            Plan::Serves => {
                send(AVAILABLE);
                pending().await
            }
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
    async fn start(sessions: Vec<Plan>) -> Reader {
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
async fn a_portal_service_that_stops_or_restarts_is_reported_and_bound_again_at_once() {
    local(async {
        for (present, why) in [
            (false, "a portal service of the desktop stopped"),
            (true, "a portal service of the desktop restarted"),
        ] {
            let r = Reader::start(Vec::new()).await;
            assert_eq!(r.take(), [AVAILABLE]);
            let start = Instant::now();
            r.owner(present).await;
            // The new session starts the service if it is gone.
            assert_eq!((r.sessions(), r.take()), (2, vec![lost(why), AVAILABLE]));
            assert_eq!(start.elapsed(), Duration::ZERO);
            assert!(!r.task.is_finished());
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_session_that_fails_is_tried_again_only_when_a_portal_service_changes() {
    local(async {
        let script = vec![Plan::Fails("declined"), Plan::BindsThenEnds("closed")];
        let r = Reader::start(script).await;
        assert_eq!(r.take(), [lost("declined")]);
        // No retry by itself, however long.
        advance(Duration::from_secs(24 * 3600)).await;
        settle().await;
        assert_eq!((r.sessions(), r.take()), (1, Vec::new()));
        r.owner(true).await;
        assert_eq!(r.sessions(), 2);
        assert_eq!(
            r.take(),
            [
                lost("a portal service of the desktop restarted"),
                AVAILABLE,
                lost("closed")
            ]
        );
        advance(Duration::from_secs(24 * 3600)).await;
        settle().await;
        assert_eq!(r.sessions(), 2);
        r.owner(false).await;
        assert_eq!(r.sessions(), 3);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_service_that_dies_on_every_bind_is_not_started_for_ever() {
    local(async {
        // Each session starts the service, which dies before the bind is
        // done: the reader hears it stop, and would start it again.
        let r = Reader::start(vec![Plan::Fails("CreateSession: the service died"); 64]).await;
        for _ in 0..64 {
            if r.task.is_finished() {
                break;
            }
            r.owner(false).await;
        }
        assert_eq!(r.sessions(), usize::try_from(MAX_FAILED_IN_A_ROW).unwrap());
        assert!(r.task.is_finished());
        let events = r.take();
        let Some(HotkeyEvent::Status(HotkeyStatus::Unavailable(Unavailable::Desktop(last)))) =
            events.last()
        else {
            panic!("{events:?}");
        };
        assert!(
            last.contains("could not be bound in 5 tries") && last.contains("restart auto-clicker"),
            "{last}"
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn sessions_cut_short_by_a_service_change_count_as_failed_too() {
    local(async {
        // Sessions that never get as far as binding, each ended by the
        // service changing hands again.
        struct NeverBinds(Rc<RefCell<usize>>);
        impl Desktop for NeverBinds {
            async fn session(&mut self, _send: &mut dyn FnMut(HotkeyEvent)) -> String {
                *self.0.borrow_mut() += 1;
                pending().await
            }
        }
        let started = Rc::new(RefCell::new(0));
        let mut desktop = NeverBinds(started.clone());
        let changes = futures_util::stream::iter(std::iter::repeat_n(true, 64));
        let mut events = Vec::new();
        run(&mut desktop, changes, |event| events.push(event)).await;
        assert_eq!(*started.borrow(), 0, "the changes were all queued first");
        assert_eq!(events.len(), usize::try_from(MAX_FAILED_IN_A_ROW).unwrap());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_session_that_bound_starts_the_count_again() {
    local(async {
        // Four failures, a session that binds, four more: never five in a
        // row, so the reader keeps going.
        let mut script = vec![Plan::Fails("no"); 4];
        script.push(Plan::BindsThenEnds("closed"));
        script.extend([Plan::Fails("no"); 4]);
        let r = Reader::start(script).await;
        for _ in 0..9 {
            r.owner(true).await;
        }
        assert_eq!(r.sessions(), 10);
        assert!(!r.task.is_finished());
        assert_eq!(r.take().last(), Some(&AVAILABLE));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_reader_ends_when_the_bus_can_no_longer_be_watched() {
    local(async {
        // While a session runs, and while waiting for the portal.
        for sessions in [Vec::new(), vec![Plan::Fails("failed")]] {
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
    assert_eq!(
        parse(&pressed),
        Some(Signal::Activated(CLICKING.to_owned()))
    );
    let let_go = signal("Deactivated", &(session_path(), RECORDING, 0u64, &options));
    assert_eq!(
        parse(&let_go),
        Some(Signal::Deactivated(RECORDING.to_owned()))
    );
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
        Some(Signal::Changed(listed(&[(CLICKING, "Ctrl+Shift+F6")])))
    );
}

#[test]
fn a_known_signal_that_does_not_parse_is_malformed_not_ignored() {
    let malformed = |name: &str| Some(Signal::Malformed(name.to_owned()));
    // An activation and a release without their shortcut id.
    let short = signal("Activated", &(session_path(),));
    assert_eq!(parse(&short), malformed("Activated"));
    let short = signal("Deactivated", &(session_path(),));
    assert_eq!(parse(&short), malformed("Deactivated"));
    let wrong = signal("ShortcutsChanged", &(session_path(), "not a list"));
    assert_eq!(parse(&wrong), malformed("ShortcutsChanged"));
    // A signal a newer portal added is none of the program's.
    let unknown = signal("Reconfigured", &(session_path(),));
    assert_eq!(parse(&unknown), None);
}

#[test]
fn a_connection_that_cannot_be_made_says_why_in_its_own_words() {
    let no_file = InputError::PortalUnavailable("could not register the app id".to_owned());
    assert_eq!(connect_failure(&no_file), "could not register the app id");
    let slow = InputError::Portal("session bus: no answer within 10 s".to_owned());
    assert_eq!(connect_failure(&slow), "session bus: no answer within 10 s");
    assert_eq!(
        connect_failure(&InputError::NotAllowed),
        InputError::NotAllowed.to_string()
    );
}

#[test]
fn the_ids_the_desktop_stores_never_change() {
    // The desktop keeps the user's triggers under these ids.
    for (action, id) in [
        (Action::ToggleClicking, "toggle_clicking"),
        (Action::ToggleRecording, "toggle_recording"),
        (Action::TogglePlayback, "toggle_playback"),
    ] {
        assert_eq!(id_of(action), id);
    }
}

#[tokio::test(start_paused = true)]
async fn a_portal_call_that_does_not_answer_times_out_and_names_the_call() {
    let unanswered = within("CreateSession", pending::<()>()).await.unwrap_err();
    assert_eq!(unanswered, "CreateSession: no answer within 10 s");
    assert_eq!(within("CreateSession", async { 7 }).await, Ok(7));
}

/// A `NameOwnerChanged` signal about `name`, sent by `sender`.
fn owner_change(sender: &str, name: &str, old: &str, new: &str) -> zbus::Message {
    zbus::Message::signal(
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameOwnerChanged",
    )
    .unwrap()
    .sender(sender)
    .unwrap()
    .build(&(name, old, new))
    .unwrap()
}

const BUS: &str = "org.freedesktop.DBus";
const KDE: &str = "org.freedesktop.impl.portal.desktop.kde";
const PORTAL: &str = "org.freedesktop.portal.Desktop";

#[test]
fn an_owner_change_says_whether_the_name_has_an_owner_now() {
    assert_eq!(has_owner(&owner_change(BUS, KDE, ":1.5", "")), Some(false));
    assert_eq!(has_owner(&owner_change(BUS, KDE, "", ":1.9")), Some(true));
    assert_eq!(
        has_owner(&owner_change(BUS, PORTAL, ":1.5", ":1.9")),
        Some(true)
    );
}

#[test]
fn an_owner_change_from_anyone_but_the_bus_or_about_another_name_is_not_one() {
    // Another program sends the signal straight to this connection.
    assert_eq!(has_owner(&owner_change(":1.77", KDE, ":1.5", "")), None);
    // The bus, about names that are neither of the two portal services.
    for name in [
        "org.freedesktop.impl.portal.unrelated",
        "org.freedesktop.impl.portal.desktop.gtk",
        "org.freedesktop.portal.Desktop.evil",
    ] {
        let change = owner_change(BUS, name, ":1.5", "");
        assert_eq!(has_owner(&change), None, "{name}");
    }
    // Not an owner change at all: unreadable, never "no owner".
    let wrong = zbus::Message::signal("/org/freedesktop/DBus", BUS, "NameOwnerChanged")
        .unwrap()
        .sender(BUS)
        .unwrap()
        .build(&(7u32,))
        .unwrap();
    assert_eq!(has_owner(&wrong), None);
}

#[test]
fn a_sessions_closed_signal_counts_only_from_the_portal_and_under_the_callers_path() {
    let namespace = session_namespace(":1.42");
    assert_eq!(namespace, "/org/freedesktop/portal/desktop/session/1_42");
    let closed = |sender: &str, path: &str| {
        let message = zbus::Message::signal(path, "org.freedesktop.portal.Session", "Closed")
            .unwrap()
            .sender(sender)
            .unwrap()
            .build(&(HashMap::<&str, Value<'_>>::new(),))
            .unwrap();
        closes_a_session(&message, ":1.7", &namespace)
    };
    let ours = "/org/freedesktop/portal/desktop/session/1_42/token";
    assert!(closed(":1.7", ours));
    // Another program, sending it straight to this connection.
    assert!(!closed(":1.99", ours));
    // The portal, about someone else's session.
    for other in ["1_43/token", "1_421/token", "1_42"] {
        let path = format!("/org/freedesktop/portal/desktop/session/{other}");
        assert!(!closed(":1.7", &path), "{path}");
    }
}

#[test]
fn a_change_listing_far_more_shortcuts_than_the_program_has_is_malformed() {
    let entry = |n: usize| {
        let info: HashMap<&str, Value<'_>> = HashMap::from([
            ("description", Value::from("x")),
            ("trigger_description", Value::from("F1")),
        ]);
        (format!("shortcut{n}"), info)
    };
    let within: Vec<_> = (0..MAX_LISTED).map(entry).collect();
    let fits = signal("ShortcutsChanged", &(session_path(), within));
    assert!(matches!(parse(&fits), Some(Signal::Changed(listed)) if listed.len() == MAX_LISTED));
    let over: Vec<_> = (0..=MAX_LISTED).map(entry).collect();
    let too_many = signal("ShortcutsChanged", &(session_path(), over));
    assert_eq!(
        parse(&too_many),
        Some(Signal::Malformed("ShortcutsChanged".to_owned()))
    );
}

#[test]
fn an_unavailable_reason_is_shown_plain_and_bounded() {
    let quoted = format!("CreateSession: \u{1b}[31m{}\n", "x".repeat(1000));
    let HotkeyEvent::Status(HotkeyStatus::Unavailable(Unavailable::Desktop(why))) =
        unavailable(quoted)
    else {
        panic!("not an unavailable status");
    };
    assert_eq!(why.chars().count(), 240);
    assert!(why.starts_with("CreateSession: [31mxxx"), "{why}");
}

#[test]
fn failed_calls_say_what_the_user_can_do() {
    use ashpd::desktop::ResponseError;
    let cancelled = failed(
        "BindShortcuts",
        &ashpd::Error::Response(ResponseError::Cancelled),
    );
    assert!(
        cancelled.contains("declined in the desktop's dialog")
            && cancelled.contains("System Settings"),
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
