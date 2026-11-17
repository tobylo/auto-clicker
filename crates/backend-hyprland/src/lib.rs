//! The pointer's position on Hyprland, for recordings (W01, decision 15).
//!
//! Hyprland answers `cursorpos` on its request socket only when asked, so
//! while a recording runs the daemon asks on each move ([`watcher`]), and
//! asks nothing while idle (brief W7). Nothing outside this crate talks to
//! Hyprland (W2).
//!
//! The source is picked once at start by what answers, never by the
//! desktop's name (W1): a socket ([`path`]) whose process is the daemon's
//! Wayland display's listener or its child, the rule KWin's backend follows
//! too. The one `HYPRLAND_INSTANCE_SIGNATURE` names is tried first, then
//! those under `$XDG_RUNTIME_DIR/hypr`, newest first, so a Hyprland the
//! daemon was not told of is still found and its own route kept: the
//! cursor sessions other compositors report through never stand in for it
//! (W02). A signature
//! that names nothing of this display leaves Hyprland out, with the reason
//! kept; none, and nothing found, is no Hyprland. The first ask at
//! start says whether Hyprland answers; one that does not still picks it,
//! since Hyprland holds its answers during a modeset, as at login.
//!
//! [`watcher`]: crate::watcher

mod ask;
mod path;
mod reply;
mod watcher;

use std::cell::Cell;
use std::ffi::OsString;
use std::future::pending;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use auto_clicker_backend_wayland::Wayland;
use auto_clicker_backend_wayland::process::{listener_or_child, parent_of};
use auto_clicker_core::controller::{LayoutSignal, LayoutWatch};
use auto_clicker_core::message::NoPointer;
use auto_clicker_core::recording::RecordingFeed;

use ask::ask;
use watcher::{Asker, Asking, Watcher};

/// Hyprland's position source as the daemon holds it. Clones share it.
#[derive(Clone)]
pub struct Hyprland(Rc<Shared>);

/// What was found at start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// No `HYPRLAND_INSTANCE_SIGNATURE`, and no socket of this display: not
    /// Hyprland.
    Absent,
    /// Hyprland of this display: it reports the pointer, or says why not.
    Picked(Result<(), NoPointer>),
    /// A signature, but nothing of this display answers on it.
    Refused(NoPointer),
    /// A socket that could not be checked (a connect that failed with
    /// someone listening, or ran late): it may be this display's Hyprland,
    /// so nothing else stands in for it (W02).
    Unsure,
}

struct Shared {
    /// The socket and the process it belongs to, when picked.
    socket: Option<(PathBuf, u32)>,
    /// What was found when nothing was picked.
    missing: Found,
    reason: Reason,
    clock: fn() -> Duration,
    watched: Cell<bool>,
}

/// Why Hyprland gives no position now, as the watcher last found; a change
/// fires the layout signal, once the watcher holds it.
#[derive(Clone, Default)]
pub(crate) struct Reason(Rc<ReasonCell>);

#[derive(Default)]
struct ReasonCell {
    now: Cell<Option<NoPointer>>,
    changed: Cell<Option<LayoutSignal>>,
}

impl Reason {
    fn get(&self) -> Result<(), NoPointer> {
        self.0.now.get().map_or(Ok(()), Err)
    }

    pub(crate) fn set(&self, reason: Result<(), NoPointer>) {
        let reason = reason.err();
        if self.0.now.replace(reason) == reason {
            return;
        }
        if let Some(why) = reason {
            eprintln!("auto-clicker: recordings get no positions from Hyprland: {why}");
        }
        let signal = self.0.changed.take();
        if let Some(signal) = &signal {
            signal.changed();
        }
        self.0.changed.set(signal);
    }

    fn tell(&self, changed: LayoutSignal) {
        self.0.changed.set(Some(changed));
    }
}

impl Hyprland {
    /// Looks for the Hyprland of `wayland`'s display, as the environment
    /// names it, with one ask; `clock` stamps positions (the event clock).
    pub async fn find(wayland: &Wayland, clock: fn() -> Duration) -> Hyprland {
        let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
        let runtime = std::env::var_os("XDG_RUNTIME_DIR");
        Hyprland::find_at(signature, runtime, wayland.compositor(), clock).await
    }

    /// The same for a signature, runtime directory and listener given.
    /// Without a signature, or with one that names no socket of this
    /// display, the sockets under the runtime directory's `hypr` are tried,
    /// newest first: a Hyprland started without the variable, or with a
    /// stale one, is still Hyprland (W02).
    pub(crate) async fn find_at(
        signature: Option<OsString>,
        runtime: Option<OsString>,
        listener: Option<NonZeroU32>,
        clock: fn() -> Duration,
    ) -> Hyprland {
        let named = signature
            .as_deref()
            .zip(runtime.as_deref())
            .and_then(|(signature, runtime)| path::socket(runtime, signature));
        let scan = runtime.as_deref().map(path::sockets).unwrap_or_default();
        let others = scan
            .sockets
            .into_iter()
            .filter(|socket| Some(socket) != named.as_ref());
        let candidates: Vec<PathBuf> = named.clone().into_iter().chain(others).collect();
        match look(candidates, listener, clock).await {
            Tried::Picked(found) => found,
            Tried::Not if scan.failed => {
                eprintln!(
                    "auto-clicker: the runtime directory's hypr could not be read, so neither \
                     Hyprland nor the display's cursor sessions give positions"
                );
                Hyprland::of(None, Found::Unsure, clock)
            }
            Tried::Unsure => {
                eprintln!(
                    "auto-clicker: a Hyprland socket could not be checked against this daemon's \
                     Wayland display, so neither Hyprland nor the display's cursor sessions give \
                     positions"
                );
                Hyprland::of(None, Found::Unsure, clock)
            }
            Tried::Not if signature.is_none() => Hyprland::of(None, Found::Absent, clock),
            Tried::Not => {
                eprintln!(
                    "auto-clicker: HYPRLAND_INSTANCE_SIGNATURE names no socket of this daemon's \
                     Wayland display, so Hyprland gives no positions"
                );
                Hyprland::of(None, Found::Refused(NoPointer::HyprlandOtherDisplay), clock)
            }
        }
    }

    /// Not Hyprland: the platform asks KWin.
    pub fn none() -> Hyprland {
        Hyprland::of(None, Found::Absent, || Duration::ZERO)
    }

    fn of(socket: Option<(PathBuf, u32)>, missing: Found, clock: fn() -> Duration) -> Hyprland {
        Hyprland(Rc::new(Shared {
            socket,
            missing,
            reason: Reason::default(),
            clock,
            watched: Cell::new(false),
        }))
    }

    /// What was found at start, and for a picked Hyprland why it gives no
    /// position now, if it does not.
    pub fn found(&self) -> Found {
        match &self.0.socket {
            Some(_) => Found::Picked(self.0.reason.get()),
            None => self.0.missing,
        }
    }

    /// The watcher, to be polled on the daemon's thread: while `feed`
    /// records, it asks Hyprland where the pointer is and feeds the answers
    /// on `wayland`'s layout, and it fires `changed` when [`Hyprland::found`]
    /// changes. Only the first call of a picked Hyprland gets it; any other
    /// watches nothing.
    pub fn watch(
        &self,
        wayland: Wayland,
        feed: RecordingFeed,
        changed: LayoutSignal,
    ) -> LayoutWatch {
        let Some((path, pid)) = self.0.socket.clone() else {
            return Box::pin(pending());
        };
        if self.0.watched.replace(true) {
            return Box::pin(pending());
        }
        self.0.reason.tell(changed);
        let watcher = Watcher {
            screens: wayland,
            asker: Socket { path, pid },
            feed,
            reason: self.0.reason.clone(),
            clock: self.0.clock,
        };
        Box::pin(watcher.run())
    }
}

/// What trying sockets found.
enum Tried {
    /// Hyprland of this display.
    Picked(Hyprland),
    /// None of this display, but one could not be checked.
    Unsure,
    /// None of this display.
    Not,
}

/// The first of `candidates` whose process is `listener` or its child,
/// else whether one could not be checked.
async fn look(
    candidates: Vec<PathBuf>,
    listener: Option<NonZeroU32>,
    clock: fn() -> Duration,
) -> Tried {
    let mut tried = Tried::Not;
    for socket in candidates {
        match try_socket(socket, listener, clock).await {
            Tried::Picked(found) => return Tried::Picked(found),
            Tried::Unsure => tried = Tried::Unsure,
            Tried::Not => {}
        }
    }
    tried
}

/// Hyprland on `socket`, picked when its process is `listener` or its
/// child, with whether it answered; not when nothing listens there or the
/// process is known to be another display's; unsure when the connect
/// failed otherwise or ran late, or the process could not be told apart
/// (its peer, its parent or the listener unknown).
async fn try_socket(
    socket: PathBuf,
    listener: Option<NonZeroU32>,
    clock: fn() -> Duration,
) -> Tried {
    let process = Cell::new(None);
    let other = Cell::new(false);
    let answer = ask(&socket, |pid| {
        let belongs =
            listener.and_then(|listener| listener_or_child(pid, parent_of(pid), listener));
        match belongs {
            Some(true) => process.set(Some(pid)),
            Some(false) => other.set(true),
            None => {}
        }
        belongs == Some(true)
    })
    .await;
    let pid = match (answer, process.get()) {
        (Err(ask::Failure::Nothing), _) => return Tried::Not,
        (Err(ask::Failure::OtherProcess), _) if other.get() => return Tried::Not,
        (_, None) => return Tried::Unsure,
        (_, Some(pid)) => pid,
    };
    let found = Hyprland::of(Some((socket, pid)), Found::Absent, clock);
    found
        .0
        .reason
        .set(answer.map(|_| ()).map_err(ask::Failure::reason));
    Tried::Picked(found)
}

/// Asks over the socket found at start, of the process found there.
struct Socket {
    path: PathBuf,
    pid: u32,
}

impl Asker for Socket {
    fn ask(&self) -> Asking {
        let (path, pid) = (self.path.clone(), self.pid);
        Box::pin(async move { ask(&path, |peer| peer == pid).await })
    }
}

#[cfg(test)]
mod tests;
