//! The GlobalShortcuts portal itself: the session, the bind and the signals.
//!
//! Each session gets a bus connection of its own, registered under the app
//! id first (the portal refuses a program without one). Dropping the
//! connection ends the session, so a failed step, a cancelled bind and a
//! lost portal all leave nothing open, without a `Close` call that could
//! fail or hang.

use std::future::{Future, ready};
use std::time::Duration;

use ashpd::PortalError;
use ashpd::desktop::global_shortcuts::{
    Activated, BindShortcutsOptions, Deactivated, GlobalShortcuts, NewShortcut, Shortcut,
    ShortcutsChanged,
};
use ashpd::desktop::{CreateSessionOptions, ResponseError, Session};
use auto_clicker_core::hotkeys::{Action, Binding, Bindings, HotkeyEvent, Modifier};
use auto_clicker_core::input::InputError;
use futures_util::{Stream, StreamExt, stream};
use tokio::time::timeout;

use super::Desktop;
use super::reactor::{Listed, Signal, bound_triggers, id_of, serve};
use crate::session::{APP_ID, CALL_TIMEOUT, DIALOG_TIMEOUT, Portal};

/// The longest list of shortcuts a signal may carry.
pub(super) const MAX_LISTED: usize = 64;

/// The bus name of the portal service.
const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
/// The bus name of Plasma's own portal service, which holds the shortcuts
/// behind the portal.
const KDE_BACKEND_NAME: &str = "org.freedesktop.impl.portal.desktop.kde";

/// Shortcut sessions on the desktop's portal.
pub(super) struct PortalShortcuts {
    bindings: Bindings,
}

impl PortalShortcuts {
    pub(super) fn new(bindings: Bindings) -> PortalShortcuts {
        PortalShortcuts { bindings }
    }
}

impl Desktop for PortalShortcuts {
    async fn session(&mut self, send: &mut dyn FnMut(HotkeyEvent)) -> String {
        match bind(self.bindings).await {
            Ok(bound) => {
                let triggers = bound_triggers(&bound.listed);
                // The session lives on the connection `bound` holds.
                serve(triggers, bound.signals, send).await
            }
            Err(why) => why,
        }
    }
}

/// A bound session: what the bind answered, and the session's signals.
struct Bound<S> {
    listed: Vec<Listed>,
    signals: S,
    _session: Session<GlobalShortcuts>,
    _portal: Portal,
}

/// Opens a session on a new connection and binds the three shortcuts.
async fn bind(bindings: Bindings) -> Result<Bound<impl Stream<Item = Signal>>, String> {
    let portal = Portal::connect()
        .await
        .map_err(|err| connect_failure(&err))?;
    let proxy = within(
        "GlobalShortcuts",
        GlobalShortcuts::with_connection(portal.connection().clone()),
    )
    .await?
    .map_err(|err| failed("GlobalShortcuts", &err))?;
    // Before the session exists, so no signal of it is missed.
    let heard = within("watching for hotkeys", proxy.receive_all_signals())
        .await?
        .map_err(|err| format!("watching for hotkeys: {err}"))?;
    // Every signal is checked against the portal's own unique name, found
    // here from the bus: neither a match rule nor the proxy's own tracking
    // of the owner holds against a signal another program sends straight
    // to this connection.
    let owner = within("finding the portal", portal_owner(portal.connection()))
        .await?
        .map_err(|err| format!("finding the portal: {err}"))?;
    let closed = within(
        "watching the session",
        session_closed(portal.connection(), owner.clone()),
    )
    .await?
    .map_err(|err| format!("watching the session: {err}"))?;
    let session = within(
        "CreateSession",
        proxy.create_session(CreateSessionOptions::default()),
    )
    .await?
    .map_err(|err| failed("CreateSession", &err))?;
    let wanted: Vec<NewShortcut> = Action::ALL
        .into_iter()
        .map(|action| new_shortcut(action, bindings))
        .collect();
    // The first bind waits for the user's answer to the desktop's dialog.
    let answer = timeout(DIALOG_TIMEOUT, async {
        proxy
            .bind_shortcuts(&session, &wanted, None, BindShortcutsOptions::default())
            .await?
            .response()
    })
    .await
    .map_err(|_| "the desktop's shortcuts dialog was left open for 5 min".to_owned())?
    .map_err(|err| failed("BindShortcuts", &err))?;
    // This connection carries one session, so every signal the portal
    // sends on it is that session's.
    let signals = stream::select(
        heard.filter_map(move |message| {
            let parsed = sent_by(&message, &owner).then(|| parse(&message));
            ready(parsed.flatten())
        }),
        closed.map(|()| Signal::Closed),
    );
    Ok(Bound {
        listed: answer.shortcuts().iter().map(listed).collect(),
        signals,
        _session: session,
        _portal: portal,
    })
}

/// The unique name of the portal service's owner, from the bus itself.
async fn portal_owner(connection: &zbus::Connection) -> zbus::Result<String> {
    let owner = zbus::fdo::DBusProxy::new(connection)
        .await?
        .get_name_owner(zbus::names::BusName::try_from(PORTAL_NAME)?)
        .await?;
    Ok(owner.as_str().to_owned())
}

/// Whether the connection with the unique name `owner` sent `message`. The
/// bus stamps the sender, so it cannot be forged.
pub(super) fn sent_by(message: &zbus::Message, owner: &str) -> bool {
    message
        .header()
        .sender()
        .is_some_and(|sender| sender.as_str() == owner)
}

/// The `Closed` signals the portal (`owner`) sends for the sessions of
/// `connection`: the one session this connection carries. Only the
/// portal's, and only for this connection's sessions, so no other program
/// on the bus can end the session by sending one.
async fn session_closed(
    connection: &zbus::Connection,
    owner: String,
) -> zbus::Result<impl Stream<Item = ()> + use<>> {
    let caller = connection.unique_name().ok_or(zbus::Error::Failure(
        "the connection has no name".to_owned(),
    ))?;
    let namespace = session_namespace(caller.as_str());
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(PORTAL_NAME)?
        .interface("org.freedesktop.portal.Session")?
        .member("Closed")?
        .path_namespace(namespace.as_str())?
        .build();
    let closed = zbus::MessageStream::for_match_rule(rule, connection, None).await?;
    Ok(closed.filter_map(move |message| {
        // A stream that fails has lost the connection: the session is gone.
        let ours = message.map_or(true, |message| {
            closes_a_session(&message, &owner, &namespace)
        });
        ready(ours.then_some(()))
    }))
}

/// Whether `message` is from the portal (the owner of its bus name, by
/// unique name) and about a session under `namespace`.
pub(super) fn closes_a_session(message: &zbus::Message, portal: &str, namespace: &str) -> bool {
    let ours = message.header().path().is_some_and(|path| {
        path.as_str()
            .strip_prefix(namespace)
            .is_some_and(|session| session.starts_with('/'))
    });
    sent_by(message, portal) && ours
}

/// Where the portal puts the sessions of the caller with this unique bus
/// name (":1.42"): under its name without the colon, dots as underscores.
pub(super) fn session_namespace(caller: &str) -> String {
    let caller = caller.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/session/{caller}")
}

fn listed(shortcut: &Shortcut) -> Listed {
    (
        shortcut.id().to_owned(),
        shortcut.trigger_description().to_owned(),
    )
}

/// One signal of the GlobalShortcuts interface, parsed once. `None` for a
/// signal the program does not know: one a newer portal added, which says
/// nothing about the shortcuts it bound.
pub(super) fn parse(message: &zbus::Message) -> Option<Signal> {
    let header = message.header();
    let name = header.member().map_or("", |member| member.as_str());
    let body = message.body();
    let parsed = match name {
        "Activated" => body
            .deserialize::<Activated>()
            .map(|signal| Signal::Activated(signal.shortcut_id().to_owned())),
        "Deactivated" => body
            .deserialize::<Deactivated>()
            .map(|signal| Signal::Deactivated(signal.shortcut_id().to_owned())),
        "ShortcutsChanged" => body.deserialize::<ShortcutsChanged>().map(|signal| {
            let shortcuts = signal.shortcuts();
            // The program has three shortcuts; a list far longer is not one
            // of its sessions' and is not worked through.
            if shortcuts.len() > MAX_LISTED {
                return Signal::Malformed(name.to_owned());
            }
            Signal::Changed(shortcuts.iter().map(listed).collect())
        }),
        _ => return None,
    };
    Some(parsed.unwrap_or_else(|_| Signal::Malformed(name.to_owned())))
}

/// What the program asks the desktop to bind for `action`.
fn new_shortcut(action: Action, bindings: Bindings) -> NewShortcut {
    let (binding, description) = match action {
        Action::ToggleClicking => (bindings.toggle_clicking, "Start or stop clicking"),
        Action::ToggleRecording => (bindings.toggle_recording, "Start or stop recording"),
        Action::TogglePlayback => (bindings.toggle_playback, "Play or stop the recording"),
    };
    let trigger = preferred_trigger(binding);
    NewShortcut::new(id_of(action), description).preferred_trigger(trigger.as_deref())
}

/// `binding` as the shortcuts specification spells a trigger ("CTRL+F6"),
/// for a binding on a function key. Other keys are named by what the
/// keyboard layout prints on them, which the program does not know; such a
/// shortcut starts without a trigger, and the user sets one in the
/// desktop's settings.
pub(super) fn preferred_trigger(binding: Binding) -> Option<String> {
    let name = binding.key().name()?;
    let number: u8 = name.strip_prefix('f')?.parse().ok()?;
    let mut trigger = String::new();
    for modifier in binding.modifiers().iter() {
        trigger.push_str(match modifier {
            Modifier::Ctrl => "CTRL+",
            Modifier::Shift => "SHIFT+",
            Modifier::Alt => "ALT+",
            Modifier::Meta => "LOGO+",
        });
    }
    trigger.push_str(&format!("F{number}"));
    Some(trigger)
}

/// Whether a portal service has an owner on the bus, each time that
/// changes, on a connection of its own that outlives every session: the
/// portal itself, and Plasma's own service behind it, which holds the
/// shortcuts. When that one restarts, the portal keeps its bus name and
/// says nothing, yet the session is dead (measured on Plasma 6.8.2). Only
/// these two names are watched, so no other name on the bus can end a
/// session. Ends when the bus goes away.
pub(super) async fn watch_owner() -> Result<impl Stream<Item = bool>, String> {
    let connection = within("session bus", async {
        zbus::connection::Builder::session()?
            .method_timeout(CALL_TIMEOUT)
            .build()
            .await
    })
    .await?
    .map_err(|err| format!("session bus: {err}"))?;
    let watching = |name: &'static str| {
        let connection = &connection;
        async move {
            within(
                "watching the portal service",
                owner_changes(connection, name),
            )
            .await?
            .map_err(|err| format!("watching the portal service: {err}"))
        }
    };
    let portal = watching(PORTAL_NAME).await?;
    let behind = watching(KDE_BACKEND_NAME).await?;
    let changes = stream::select(portal, behind)
        // The stream keeps the connection it listens on, and ends when the
        // connection fails.
        .scan(connection, |_connection, change| ready(change.ok()))
        // A signal that is not the bus's own word about one of the two
        // names says nothing: it is left out, and the watch goes on.
        .filter_map(|change| ready(has_owner(&change)));
    Ok(changes)
}

/// The bus's own `NameOwnerChanged` signals for `name`.
async fn owner_changes(
    connection: &zbus::Connection,
    name: &'static str,
) -> zbus::Result<zbus::MessageStream> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, name)?
        .build();
    zbus::MessageStream::for_match_rule(rule, connection, None).await
}

/// Whether the name a `NameOwnerChanged` signal is about has an owner now;
/// `None` when the signal cannot be read, is not the bus's own, or is about
/// another name. The bus alone sends as `org.freedesktop.DBus`, and that is
/// checked here: a signal sent straight to this connection arrives whatever
/// the rule says.
pub(super) fn has_owner(change: &zbus::Message) -> Option<bool> {
    let from_the_bus = change
        .header()
        .sender()
        .is_some_and(|sender| sender.as_str() == "org.freedesktop.DBus");
    let (name, _old, new): (String, String, String) = change.body().deserialize().ok()?;
    let watched = [PORTAL_NAME, KDE_BACKEND_NAME].contains(&name.as_str());
    (from_the_bus && watched).then_some(!new.is_empty())
}

/// Why the registered connection could not be made, without the wording of
/// the remote input portal it is shared with.
pub(super) fn connect_failure(err: &InputError) -> String {
    match err {
        InputError::PortalUnavailable(why) | InputError::Portal(why) => why.clone(),
        other => other.to_string(),
    }
}

/// Runs a portal call with [`CALL_TIMEOUT`].
pub(super) async fn within<T>(what: &str, call: impl Future<Output = T>) -> Result<T, String> {
    timeout(CALL_TIMEOUT, call)
        .await
        .map_err(|_| unanswered(what, CALL_TIMEOUT))
}

fn unanswered(what: &str, after: Duration) -> String {
    format!("{what}: no answer within {} s", after.as_secs())
}

/// What a failed portal call means for the user.
pub(super) fn failed(call: &str, err: &ashpd::Error) -> String {
    match err {
        ashpd::Error::Response(ResponseError::Cancelled) => {
            "the shortcuts were declined in the desktop's dialog; restart auto-clicker and set \
             them in System Settings, under Shortcuts"
                .to_owned()
        }
        ashpd::Error::Portal(PortalError::NotAllowed(why)) => format!(
            "the desktop's shortcuts portal refused the program ({why}); is the package's \
             {APP_ID}.desktop file installed?"
        ),
        ashpd::Error::PortalNotFound(_) => "the desktop has no GlobalShortcuts portal".to_owned(),
        other => format!("{call}: {other}"),
    }
}
