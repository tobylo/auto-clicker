//! The user's graphical session, from logind on the system bus: whether it is
//! locked, in front and awake.
//!
//! The daemon runs as a user service, outside the session it serves, so it
//! follows the user's `Display` session and switches when that changes.

use auto_clicker_core::controller::{Blocked, Message, SessionFacts};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use zbus::Connection;
use zbus::proxy::PropertyStream;
use zbus::zvariant::OwnedObjectPath;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.User",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1/user/self"
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

/// Sends the session's state to the controller whenever it changes. When
/// logind cannot be followed any more, says why: the controller then refuses
/// to start, since a state it cannot read is not "unlocked".
pub async fn watch(send: mpsc::UnboundedSender<Message>) {
    let reason = match follow(&send).await {
        Ok(()) => "logind stopped sending changes".to_owned(),
        Err(err) => format!("logind: {err}"),
    };
    let _ = send.send(Message::Session(Some(Blocked::Unknown(reason))));
}

/// Something logind said, or the end of a stream (`None`).
enum Change {
    Sleep(bool),
    Locked(bool),
    Active(bool),
    LockRequested,
    Display(OwnedObjectPath),
}

/// The streams of one session.
struct SessionStreams {
    locks: LockStream,
    locked: PropertyStream<'static, bool>,
    active: PropertyStream<'static, bool>,
}

/// Follows logind until a stream ends, a call fails or the controller is
/// gone.
struct Follower<'a> {
    bus: Connection,
    send: &'a mpsc::UnboundedSender<Message>,
    facts: SessionFacts,
    sleeps: PrepareForSleepStream,
    /// Yields the current session first, then each change.
    displays: PropertyStream<'static, (String, OwnedObjectPath)>,
}

async fn follow(send: &mpsc::UnboundedSender<Message>) -> zbus::Result<()> {
    let bus = Connection::system().await?;
    let sleeps = ManagerProxy::new(&bus)
        .await?
        .receive_prepare_for_sleep()
        .await?;
    let displays = UserProxy::new(&bus).await?.receive_display_changed().await;
    let mut follower = Follower {
        bus,
        send,
        facts: SessionFacts::default(),
        sleeps,
        displays,
    };
    let mut display = follower.next_display().await?;
    while let Some(path) = display {
        display = follower.follow_display(path).await?;
    }
    Ok(())
}

impl Follower<'_> {
    /// The next `Display` session, or `None` when the stream ended.
    async fn next_display(&mut self) -> zbus::Result<Option<OwnedObjectPath>> {
        match self.displays.next().await {
            Some(change) => Ok(Some(change.get().await?.1)),
            None => Ok(None),
        }
    }

    /// Follows `path` until the user's display session changes; the new one.
    async fn follow_display(
        &mut self,
        path: OwnedObjectPath,
    ) -> zbus::Result<Option<OwnedObjectPath>> {
        // logind's `Display` is ("", "/") when the user has none.
        if path.as_str() == "/" {
            let reason = "no graphical session".to_owned();
            self.tell(Message::Session(Some(Blocked::Unknown(reason))));
            return self.next_display().await;
        }
        let session = SessionProxy::builder(&self.bus)
            .path(path.clone())?
            .build()
            .await?;
        let mut streams = SessionStreams {
            locks: session.receive_lock().await?,
            // Each yields the current value first.
            locked: session.receive_locked_hint_changed().await,
            active: session.receive_active_changed().await,
        };
        self.facts.locked = session.locked_hint().await?;
        self.facts.active = session.active().await?;
        self.tell(Message::Session(self.facts.blocked()));
        loop {
            let message = match self.next_change(&mut streams).await? {
                None => return Ok(None),
                Some(Change::Display(new)) if new == path => continue,
                Some(Change::Display(new)) => return Ok(Some(new)),
                Some(change) => self.apply(change),
            };
            if !self.tell(message) {
                return Ok(None);
            }
        }
    }

    /// The next thing logind says; `None` when a stream ended.
    async fn next_change(&mut self, streams: &mut SessionStreams) -> zbus::Result<Option<Change>> {
        tokio::select! {
            sleep = self.sleeps.next() => sleep.map(|s| s.args().map(|a| Change::Sleep(a.start))).transpose(),
            lock = streams.locks.next() => Ok(lock.map(|_| Change::LockRequested)),
            hint = streams.locked.next() => property(hint, Change::Locked).await,
            now = streams.active.next() => property(now, Change::Active).await,
            display = self.displays.next() => match display {
                Some(change) => Ok(Some(Change::Display(change.get().await?.1))),
                None => Ok(None),
            },
        }
    }

    /// The message for a change of the session's state.
    fn apply(&mut self, change: Change) -> Message {
        match change {
            Change::Sleep(sleeping) => self.facts.sleeping = sleeping,
            Change::Locked(locked) => self.facts.locked = locked,
            Change::Active(active) => self.facts.active = active,
            Change::LockRequested => return Message::LockRequested,
            // Handled by the caller.
            Change::Display(_) => {}
        }
        Message::Session(self.facts.blocked())
    }

    /// Sends `message`; `false` when the controller is gone.
    fn tell(&self, message: Message) -> bool {
        self.send.send(message).is_ok()
    }
}

/// A property's new value as a change, `None` when its stream ended.
async fn property(
    changed: Option<zbus::proxy::PropertyChanged<'static, bool>>,
    change: fn(bool) -> Change,
) -> zbus::Result<Option<Change>> {
    match changed {
        Some(changed) => Ok(Some(change(changed.get().await?))),
        None => Ok(None),
    }
}
