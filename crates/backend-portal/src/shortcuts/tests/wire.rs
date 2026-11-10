//! The portal's side: what is asked of it, and how its signals and errors
//! are read.

use super::*;

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
