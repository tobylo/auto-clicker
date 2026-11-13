//! The pointer's position on Hyprland, for recordings (W01, decision 15).
//!
//! Hyprland answers `cursorpos` on its request socket only when asked, so
//! while a recording runs the daemon asks on each move ([`watcher`]), and
//! asks nothing while idle (brief W7). Nothing outside this crate talks to
//! Hyprland (W2).
//!
//! The source is picked once at start by what answers, never by the
//! desktop's name (W1): `HYPRLAND_INSTANCE_SIGNATURE` must name a socket
//! ([`path`]) whose process is the daemon's Wayland display's listener or
//! its child, the rule KWin's backend follows too. A signature that names
//! nothing usable, a socket of another display, or no display to check
//! against leaves Hyprland out, with the reason kept. The first ask at
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
    /// No `HYPRLAND_INSTANCE_SIGNATURE`: not Hyprland, or one the user's
    /// session did not tell the daemon of.
    Absent,
    /// Hyprland of this display: it reports the pointer, or says why not.
    Picked(Result<(), NoPointer>),
    /// A signature, but nothing of this display answers on it.
    Refused(NoPointer),
}

struct Shared {
    /// The socket and the process it belongs to, when picked.
    socket: Option<(PathBuf, u32)>,
    refused: Option<NoPointer>,
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
    pub(crate) async fn find_at(
        signature: Option<OsString>,
        runtime: Option<OsString>,
        listener: Option<NonZeroU32>,
        clock: fn() -> Duration,
    ) -> Hyprland {
        let Some(signature) = signature else {
            return Hyprland::of(None, None, clock);
        };
        let socket = runtime.and_then(|runtime| path::socket(&runtime, &signature));
        let (Some(socket), Some(listener)) = (socket, listener) else {
            eprintln!(
                "auto-clicker: HYPRLAND_INSTANCE_SIGNATURE names no socket this daemon can check \
                 against its Wayland display, so Hyprland gives no positions"
            );
            return Hyprland::of(None, Some(NoPointer::HyprlandOtherDisplay), clock);
        };
        let process = Cell::new(None);
        let answer = ask(&socket, |pid| {
            process.set(Some(pid));
            listener_or_child(pid, parent_of(pid), listener) == Some(true)
        })
        .await;
        match (answer, process.get()) {
            (Err(ask::Failure::OtherProcess), _) | (_, None) => {
                eprintln!(
                    "auto-clicker: Hyprland's socket is not this display's, so Hyprland gives \
                     no positions"
                );
                Hyprland::of(None, Some(NoPointer::HyprlandOtherDisplay), clock)
            }
            (answer, Some(pid)) => {
                let found = Hyprland::of(Some((socket, pid)), None, clock);
                found
                    .0
                    .reason
                    .set(answer.map(|_| ()).map_err(ask::Failure::reason));
                found
            }
        }
    }

    /// Not Hyprland: the platform asks KWin.
    pub fn none() -> Hyprland {
        Hyprland::of(None, None, || Duration::ZERO)
    }

    fn of(
        socket: Option<(PathBuf, u32)>,
        refused: Option<NoPointer>,
        clock: fn() -> Duration,
    ) -> Hyprland {
        Hyprland(Rc::new(Shared {
            socket,
            refused,
            reason: Reason::default(),
            clock,
            watched: Cell::new(false),
        }))
    }

    /// What was found at start, and for a picked Hyprland why it gives no
    /// position now, if it does not.
    pub fn found(&self) -> Found {
        match (&self.0.socket, self.0.refused) {
            (Some(_), _) => Found::Picked(self.0.reason.get()),
            (None, Some(why)) => Found::Refused(why),
            (None, None) => Found::Absent,
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
