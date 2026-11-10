//! The user's graphical session, from logind on the system bus: whether it is
//! locked, in front and awake.
//!
//! The daemon runs as a user service, outside the session it serves, so it
//! follows the user's `Display` session and switches when that changes.
//! Changes are read from logind's own signals, in order, with the values
//! they carry: a session that goes inactive and back stops clicking, however
//! fast it comes back. While logind is gone (restarting), starts are refused.

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use auto_clicker_core::controller::{Blocked, Message, SessionFacts};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use zbus::fdo::{
    DBusProxy, NameOwnerChangedStream, PropertiesChanged, PropertiesChangedStream, PropertiesProxy,
};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};
use zbus::{Connection, connection};

const LOGIND: &str = "org.freedesktop.login1";
const SESSION_INTERFACE: &str = "org.freedesktop.login1.Session";
const USER_INTERFACE: &str = "org.freedesktop.login1.User";
/// The system bus, by its fixed socket: the environment cannot point the
/// daemon at another bus or another transport.
const SYSTEM_BUS: &str = "unix:path=/run/dbus/system_bus_socket";
/// How long logind may take to answer any call.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn preparing_for_sleep(&self) -> zbus::Result<bool>;

    #[zbus(name = "GetUserByPID")]
    fn get_user_by_pid(&self, pid: u32) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.User",
    default_service = "org.freedesktop.login1"
)]
trait User {
    #[zbus(property)]
    fn display(&self) -> zbus::Result<(String, OwnedObjectPath)>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait Session {
    #[zbus(signal)]
    fn lock(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
}

/// Sends the session's state to the controller whenever it changes, until
/// the controller is gone. When logind cannot be followed, says why and
/// stops: the controller then refuses to start, since a state it cannot read
/// is not "unlocked".
pub async fn watch(send: mpsc::UnboundedSender<Message>) {
    if let Err(err) = watch_logind(&send).await {
        let reason = format!("logind: {err}");
        let _ = send.send(Message::Session(Some(Blocked::Unknown(reason))));
    }
}

/// Something logind said.
#[derive(Debug, PartialEq)]
enum Change {
    Sleep(bool),
    Locked(bool),
    Active(bool),
    LockRequested,
    Display(OwnedObjectPath),
    /// logind's name changed owner: it left the bus, or another process
    /// took the name; `back` when a new owner already has it.
    LogindGone {
        back: bool,
    },
}

/// How following one logind instance ended without an error.
enum End {
    ControllerGone,
    /// `back` when a new logind already owns the name.
    LogindGone {
        back: bool,
    },
}

/// Follows logind until the controller is gone. Whatever stops it (logind
/// restarting, a call that failed, a session it could not read), starts are
/// refused with the reason until logind next changes owner, and then it is
/// followed afresh. Nothing is retried on a timer.
async fn watch_logind(send: &mpsc::UnboundedSender<Message>) -> zbus::Result<()> {
    let (bus, mut owners) = bounded("connecting", connect()).await?;
    loop {
        let (reason, back) = match follow_once(&bus, send, &mut owners).await {
            Ok(End::ControllerGone) => return Ok(()),
            Ok(End::LogindGone { back }) => ("logind is restarting".to_owned(), back),
            Err(err) => (format!("logind: {err}"), false),
        };
        if send
            .send(Message::Session(Some(Blocked::Unknown(reason))))
            .is_err()
        {
            return Ok(());
        }
        if !back {
            wait_for_owner(&mut owners).await?;
        }
    }
}

/// The system bus, and logind's name changes, subscribed to before anything
/// is read so no restart goes unseen.
async fn connect() -> zbus::Result<(Connection, NameOwnerChangedStream)> {
    let bus = connection::Builder::address(SYSTEM_BUS)?
        .method_timeout(CALL_TIMEOUT)
        .build()
        .await?;
    let owners = DBusProxy::new(&bus)
        .await?
        .receive_name_owner_changed_with_args(&[(0, LOGIND)])
        .await?;
    Ok((bus, owners))
}

/// `step`, given `CALL_TIMEOUT`: zbus's method timeout bounds method calls,
/// but not connecting or subscribing to signals.
async fn bounded<T>(what: &str, step: impl Future<Output = zbus::Result<T>>) -> zbus::Result<T> {
    tokio::time::timeout(CALL_TIMEOUT, step)
        .await
        .map_err(|_| zbus::Error::Failure(format!("no answer while {what}")))?
}

/// Subscribes to one logind instance and follows it.
async fn follow_once(
    bus: &Connection,
    send: &mpsc::UnboundedSender<Message>,
    owners: &mut NameOwnerChangedStream,
) -> zbus::Result<End> {
    bounded("subscribing", Logind::subscribe(bus, send))
        .await?
        .follow(owners)
        .await
}

/// Waits until logind is on the bus again.
async fn wait_for_owner(owners: &mut NameOwnerChangedStream) -> zbus::Result<()> {
    while let Some(change) = owners.next().await {
        if change.args()?.new_owner().is_some() {
            return Ok(());
        }
    }
    Err(ended("logind's name changes"))
}

fn ended(what: &str) -> zbus::Error {
    zbus::Error::Failure(format!("the system bus stopped sending {what}"))
}

/// The streams of the session being followed.
struct SessionStreams {
    path: OwnedObjectPath,
    locks: LockStream,
    properties: PropertiesChangedStream,
}

/// Everything subscribed to on one logind instance.
struct Logind<'a> {
    bus: Connection,
    send: &'a mpsc::UnboundedSender<Message>,
    facts: SessionFacts,
    sleeps: PrepareForSleepStream,
    /// The user's properties: `Display` changes.
    user: PropertiesChangedStream,
    /// `None` while the user has no graphical session.
    session: Option<SessionStreams>,
}

impl<'a> Logind<'a> {
    /// Subscribes to everything, then reads the current state and tells the
    /// controller. Signals are subscribed to before the state is read, so
    /// none is missed in between.
    async fn subscribe(
        bus: &Connection,
        send: &'a mpsc::UnboundedSender<Message>,
    ) -> zbus::Result<Logind<'a>> {
        let manager = ManagerProxy::builder(bus)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let sleeps = manager.receive_prepare_for_sleep().await?;
        let user_path = manager.get_user_by_pid(std::process::id()).await?;
        let user = properties(bus, &user_path).await?;
        let facts = SessionFacts {
            sleeping: manager.preparing_for_sleep().await?,
            ..SessionFacts::default()
        };
        let display = UserProxy::builder(bus)
            .path(user_path)?
            .cache_properties(CacheProperties::No)
            .build()
            .await?
            .display()
            .await?
            .1;
        let mut logind = Logind {
            bus: bus.clone(),
            send,
            facts,
            sleeps,
            user,
            session: None,
        };
        logind.switch_to(display).await?;
        Ok(logind)
    }

    /// Follows the session at `path` from now on, and tells the controller
    /// its state; `false` when the controller is gone.
    async fn switch_to(&mut self, path: OwnedObjectPath) -> zbus::Result<bool> {
        // The old session's unread signals go with its streams, so whatever
        // they said, clicking stops here, and stays stopped until the new
        // session says otherwise.
        let left = leave_session(&mut self.facts);
        self.session = None;
        if !self.tell(left) {
            return Ok(false);
        }
        if is_no_session(&path) {
            let reason = "no graphical session".to_owned();
            return Ok(self.tell(Message::Session(Some(Blocked::Unknown(reason)))));
        }
        let properties = properties(&self.bus, &path).await?;
        let session = SessionProxy::builder(&self.bus)
            .path(path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let locks = session.receive_lock().await?;
        self.facts.locked = session.locked_hint().await?;
        self.facts.active = session.active().await?;
        self.session = Some(SessionStreams {
            path,
            locks,
            properties,
        });
        Ok(self.tell(Message::Session(self.facts.blocked())))
    }

    /// Follows logind until it leaves the bus or the controller is gone.
    async fn follow(&mut self, owners: &mut NameOwnerChangedStream) -> zbus::Result<End> {
        loop {
            for change in self.next_changes(owners).await? {
                let path = match &self.session {
                    Some(session) => session.path.clone(),
                    None => no_session(),
                };
                let told = match step(&mut self.facts, &path, change) {
                    Step::Tell(message) => self.tell(message),
                    Step::Switch(path) => {
                        bounded("switching sessions", self.switch_to(path)).await?
                    }
                    Step::LogindGone { back } => return Ok(End::LogindGone { back }),
                    Step::Ignore => true,
                };
                if !told {
                    return Ok(End::ControllerGone);
                }
            }
        }
    }

    /// What logind says next, in the order it said it.
    async fn next_changes(
        &mut self,
        owners: &mut NameOwnerChangedStream,
    ) -> zbus::Result<Vec<Change>> {
        let (locks, session) = match self.session.as_mut() {
            Some(streams) => (Some(&mut streams.locks), Some(&mut streams.properties)),
            None => (None, None),
        };
        tokio::select! {
            owner = owners.next() => {
                let owner = owner.ok_or_else(|| ended("logind's name changes"))?;
                let back = owner.args()?.new_owner().is_some();
                Ok(vec![Change::LogindGone { back }])
            }
            sleep = self.sleeps.next() => {
                let sleep = sleep.ok_or_else(|| ended("sleep signals"))?;
                Ok(vec![Change::Sleep(sleep.args()?.start)])
            }
            changed = self.user.next() => {
                user_changes(&changed.ok_or_else(|| ended("user changes"))?)
            }
            lock = next_of(locks) => {
                lock.ok_or_else(|| ended("lock signals"))?;
                Ok(vec![Change::LockRequested])
            }
            changed = next_of(session) => {
                session_changes(&changed.ok_or_else(|| ended("session changes"))?)
            }
        }
    }

    /// Sends `message`; `false` when the controller is gone.
    fn tell(&self, message: Message) -> bool {
        self.send.send(message).is_ok()
    }
}

/// The `PropertiesChanged` signal of the logind object at `path`.
async fn properties(
    bus: &Connection,
    path: &OwnedObjectPath,
) -> zbus::Result<PropertiesChangedStream> {
    PropertiesProxy::builder(bus)
        .destination(LOGIND)?
        .path(path.clone())?
        .cache_properties(CacheProperties::No)
        .build()
        .await?
        .receive_properties_changed()
        .await
}

/// The next item of `stream`; never resolves without a stream.
async fn next_of<S: futures_util::Stream + Unpin>(stream: Option<&mut S>) -> Option<S::Item> {
    match stream {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

fn session_changes(changed: &PropertiesChanged) -> zbus::Result<Vec<Change>> {
    interface_changes(changed, SESSION_INTERFACE, &["Active", "LockedHint"])
}

fn user_changes(changed: &PropertiesChanged) -> zbus::Result<Vec<Change>> {
    interface_changes(changed, USER_INTERFACE, &["Display"])
}

/// The changes a `PropertiesChanged` of `interface` carries. A watched
/// property it only invalidates, without a value, is an error: logind always
/// sends values, and a value not sent is not known.
fn interface_changes(
    changed: &PropertiesChanged,
    interface: &str,
    watched: &[&str],
) -> zbus::Result<Vec<Change>> {
    let args = changed.args()?;
    if args.interface_name().as_str() != interface {
        return Ok(Vec::new());
    }
    refuse_invalidated(args.invalidated_properties(), watched)?;
    property_changes(args.changed_properties(), watched)
}

fn refuse_invalidated(invalidated: &[&str], watched: &[&str]) -> zbus::Result<()> {
    match invalidated.iter().find(|name| watched.contains(name)) {
        Some(name) => Err(zbus::Error::Failure(format!(
            "logind sent no value for {name}"
        ))),
        None => Ok(()),
    }
}

/// The `watched` properties among `changed`, as changes; others are
/// ignored, whatever they are called (a session's `Display` is the X display
/// string, not the user's session). Active comes before LockedHint, so a
/// session that went inactive is stopped as such.
fn property_changes(
    changed: &HashMap<&str, Value<'_>>,
    watched: &[&str],
) -> zbus::Result<Vec<Change>> {
    let get = |name: &str| watched.contains(&name).then(|| changed.get(name)).flatten();
    let mut changes = Vec::new();
    if let Some(value) = get("Active") {
        changes.push(Change::Active(bool::try_from(value)?));
    }
    if let Some(value) = get("LockedHint") {
        changes.push(Change::Locked(bool::try_from(value)?));
    }
    if let Some(value) = get("Display") {
        let (_, path) = <(String, OwnedObjectPath)>::try_from(value.try_clone()?)?;
        changes.push(Change::Display(path));
    }
    Ok(changes)
}

/// Leaves the followed session: until another one is read, the user has no
/// session in front. The message that stops clicking.
fn leave_session(facts: &mut SessionFacts) -> Message {
    facts.active = false;
    Message::Session(facts.blocked())
}

/// What to do about a change while following the session at `path`.
#[derive(Debug, PartialEq)]
enum Step {
    Tell(Message),
    /// The user's display session is another one now: follow it instead.
    Switch(OwnedObjectPath),
    LogindGone {
        back: bool,
    },
    Ignore,
}

/// Folds `change` into `facts`, and says what follows from it.
fn step(facts: &mut SessionFacts, path: &OwnedObjectPath, change: Change) -> Step {
    match change {
        Change::Display(new) if new == *path => return Step::Ignore,
        Change::Display(new) => return Step::Switch(new),
        Change::LogindGone { back } => return Step::LogindGone { back },
        Change::LockRequested => return Step::Tell(Message::LockRequested),
        Change::Sleep(sleeping) => facts.sleeping = sleeping,
        Change::Locked(locked) => facts.locked = locked,
        Change::Active(active) => facts.active = active,
    }
    Step::Tell(Message::Session(facts.blocked()))
}

/// logind's `Display` path when the user has no graphical session.
fn no_session() -> OwnedObjectPath {
    OwnedObjectPath::from(ObjectPath::from_static_str_unchecked("/"))
}

fn is_no_session(path: &OwnedObjectPath) -> bool {
    *path == no_session()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(text).unwrap()
    }

    #[test]
    fn each_fact_change_tells_the_whole_state() {
        let here = path("/org/freedesktop/login1/session/_31");
        let mut facts = SessionFacts {
            active: true,
            ..SessionFacts::default()
        };
        let mut tell = |change| step(&mut facts, &here, change);
        let session = |blocked| Step::Tell(Message::Session(blocked));
        assert_eq!(tell(Change::Locked(true)), session(Some(Blocked::Locked)));
        assert_eq!(tell(Change::Sleep(true)), session(Some(Blocked::Sleeping)));
        assert_eq!(tell(Change::Sleep(false)), session(Some(Blocked::Locked)));
        assert_eq!(tell(Change::Locked(false)), session(None));
        assert_eq!(
            tell(Change::Active(false)),
            session(Some(Blocked::Inactive))
        );
        assert_eq!(
            tell(Change::LockRequested),
            Step::Tell(Message::LockRequested)
        );
        let gone = Change::LogindGone { back: true };
        assert_eq!(tell(gone), Step::LogindGone { back: true });
    }

    #[test]
    fn leaving_a_session_blocks_until_another_is_read() {
        let mut facts = SessionFacts {
            active: true,
            ..SessionFacts::default()
        };
        let left = leave_session(&mut facts);
        assert_eq!(left, Message::Session(Some(Blocked::Inactive)));
        // A resume while no session is followed still blocks.
        let resumed = step(&mut facts, &no_session(), Change::Sleep(false));
        assert_eq!(
            resumed,
            Step::Tell(Message::Session(Some(Blocked::Inactive)))
        );
    }

    #[test]
    fn only_another_display_session_switches() {
        let here = path("/org/freedesktop/login1/session/_31");
        let there = path("/org/freedesktop/login1/session/_32");
        let mut facts = SessionFacts::default();
        let same = step(&mut facts, &here, Change::Display(here.clone()));
        assert_eq!(same, Step::Ignore);
        let other = step(&mut facts, &here, Change::Display(there.clone()));
        assert_eq!(other, Step::Switch(there));
    }

    #[test]
    fn the_root_path_means_no_session() {
        assert!(is_no_session(&path("/")));
        assert!(!is_no_session(&path("/org/freedesktop/login1/session/_31")));
    }

    const SESSION: [&str; 2] = ["Active", "LockedHint"];

    #[test]
    fn a_session_change_becomes_changes_in_order_and_ignores_the_rest() {
        let changed = HashMap::from([
            ("LockedHint", Value::from(true)),
            ("Active", Value::from(false)),
            ("IdleHint", Value::from(true)),
            // A session's own Display: the X display name, a string.
            ("Display", Value::from(":0")),
        ]);
        assert_eq!(
            property_changes(&changed, &SESSION).unwrap(),
            [Change::Active(false), Change::Locked(true)]
        );
    }

    #[test]
    fn a_user_change_carries_the_display_session() {
        let session = path("/org/freedesktop/login1/session/_32");
        let display = Value::from(("2".to_owned(), session.clone()));
        let changed = HashMap::from([("Display", display), ("State", Value::from("active"))]);
        assert_eq!(
            property_changes(&changed, &["Display"]).unwrap(),
            [Change::Display(session)]
        );
    }

    #[test]
    fn a_value_of_the_wrong_type_is_an_error() {
        let changed = HashMap::from([("Active", Value::from("yes"))]);
        assert!(property_changes(&changed, &SESSION).is_err());
    }

    #[test]
    fn an_invalidated_watched_property_is_an_error() {
        let watched = ["Active", "LockedHint"];
        assert!(refuse_invalidated(&["IdleHint"], &watched).is_ok());
        assert!(refuse_invalidated(&["LockedHint"], &watched).is_err());
    }
}
