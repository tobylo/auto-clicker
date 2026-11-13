//! Whether KWin is the compositor of the daemon's Wayland display (brief
//! W1): another session's KWin on the same bus, or the KWin a nested
//! compositor runs inside, must not give its layout or positions.
//!
//! Told by process, not by name: KWin counts when the process that owns
//! `org.kde.KWin` is the one listening on the display's socket, or its
//! child. On Plasma 6.8 `kwin_wayland_wrapper` listens and keeps the socket
//! across KWin restarts; `kwin_wayland`, its child, owns the name (decision
//! 0004). Each new owner is checked before anything is loaded into it.
//!
//! With nothing to check against (no display, or its listener unknown) KWin
//! counts, as before W01. A lookup that fails, or a parent that cannot be
//! read, is tried again after the watcher's back-off, and a KWin that still
//! cannot be told then counts too: a failed lookup never takes the layout
//! away for KWin's life. A newer owner drops the check of the one before.

use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use futures_util::Stream;

use auto_clicker_backend_wayland::process::listener_or_child;
pub(crate) use auto_clicker_backend_wayland::process::{Parent, parent_of};

use crate::scripting::{Owners, Scripting, bounded};
use crate::session::Session;
use crate::watcher::BACK_OFF;

/// A process as the check needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Process {
    pub(crate) pid: u32,
    pub(crate) parent: Parent,
}

/// `owner` when its KWin counts, else `None`.
pub(crate) async fn checked<S: Scripting>(
    session: &Session<S>,
    owner: Option<String>,
) -> Option<String> {
    let (Some(owner), Some(compositor)) = (owner.clone(), session.compositor) else {
        return owner;
    };
    on_display(&session.scripting, &owner, compositor)
        .await
        .then_some(owner)
}

/// `owner` when its KWin counts, asked once: at exit, where a lookup that
/// fails leaves KWin alone and the next start cleans up.
pub(crate) async fn checked_once<S: Scripting>(
    session: &Session<S>,
    owner: Option<String>,
) -> Option<String> {
    let (Some(owner), Some(compositor)) = (owner.clone(), session.compositor) else {
        return owner;
    };
    let told = match bounded(session.scripting.owner_process(&owner)).await {
        Ok(process) => {
            counts(process, compositor).ok_or_else(|| "its parent is unknown".to_owned())
        }
        Err(err) => Err(err.0),
    };
    match told {
        Ok(ours) => ours.then_some(owner),
        Err(why) => {
            eprintln!(
                "auto-clicker: cannot tell whether KWin runs this display, so its scripts are \
                 left to the next start: {why}"
            );
            None
        }
    }
}

/// `owners` with each checked: a new owner reads as none until it counts,
/// so nothing is loaded into a KWin before it is checked, and a newer owner
/// drops the check of the one before.
pub(crate) fn each_checked<S: Scripting + 'static>(
    session: Rc<Session<S>>,
    owners: Owners,
) -> Owners {
    if session.compositor.is_none() {
        return owners;
    }
    Box::pin(Latest {
        session,
        owners: Some(owners),
        check: None,
    })
}

/// The owners, each checked, the newest only.
struct Latest<S> {
    session: Rc<Session<S>>,
    /// `None` once the bus ended it.
    owners: Option<Owners>,
    /// The newest owner's check, until it ends.
    check: Option<Pin<Box<dyn Future<Output = Option<String>>>>>,
}

impl<S: Scripting + 'static> Stream for Latest<S> {
    type Item = Option<String>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let latest = &mut *self;
        if let Some(owner) = latest.next_owner(cx) {
            let session = Rc::clone(&latest.session);
            latest.check = Some(Box::pin(async move { checked(&session, owner).await }));
            return Poll::Ready(Some(None));
        }
        if let Some(check) = &mut latest.check {
            let told = check.as_mut().poll(cx);
            if told.is_ready() {
                latest.check = None;
            }
            return told.map(Some);
        }
        if latest.owners.is_none() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

impl<S> Latest<S> {
    /// A new owner, if one came; notes the end of the owners.
    fn next_owner(&mut self, cx: &mut Context<'_>) -> Option<Option<String>> {
        let owners = self.owners.as_mut()?;
        match owners.as_mut().poll_next(cx) {
            Poll::Ready(Some(owner)) => Some(owner),
            Poll::Ready(None) => {
                self.owners = None;
                None
            }
            Poll::Pending => None,
        }
    }
}

/// Whether `owner`'s process is `compositor` or its child.
async fn on_display<S: Scripting>(scripting: &S, owner: &str, compositor: NonZeroU32) -> bool {
    let waits = std::iter::once(None).chain(BACK_OFF.iter().copied().map(Some));
    for wait in waits {
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
        }
        let process = bounded(scripting.owner_process(owner)).await;
        match process.map(|process| counts(process, compositor)) {
            Ok(Some(ours)) => return ours,
            Ok(None) => eprintln!(
                "auto-clicker: cannot tell whether KWin runs this display: its parent is unknown"
            ),
            Err(err) => eprintln!(
                "auto-clicker: cannot tell whether KWin runs this display: {}",
                err.0
            ),
        }
    }
    eprintln!("auto-clicker: KWin is taken to run this display, as it cannot be told");
    true
}

/// `None` when it cannot be told: the process is not the listener and its
/// parent is unknown.
fn counts(process: Process, compositor: NonZeroU32) -> Option<bool> {
    let ours = listener_or_child(process.pid, process.parent, compositor)?;
    if !ours {
        eprintln!(
            "auto-clicker: KWin runs another display than this daemon's, so its layout and \
             positions are not used"
        );
    }
    Some(ours)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compositor(pid: u32) -> NonZeroU32 {
        NonZeroU32::new(pid).unwrap()
    }

    #[test]
    fn kwin_counts_as_the_listener_or_its_child_only() {
        let cases = [
            // kwin_wayland under kwin_wayland_wrapper, as on Plasma 6.8.
            (
                Process {
                    pid: 1491,
                    parent: Parent::Of(1487),
                },
                1487,
                Some(true),
            ),
            // A KWin that listens on its socket itself.
            (
                Process {
                    pid: 1487,
                    parent: Parent::Of(1),
                },
                1487,
                Some(true),
            ),
            // Another session's KWin, or one under a nested compositor.
            (
                Process {
                    pid: 2210,
                    parent: Parent::Of(2201),
                },
                1487,
                Some(false),
            ),
            (
                Process {
                    pid: 2210,
                    parent: Parent::Gone,
                },
                1487,
                Some(false),
            ),
            // A parent that could not be read: not told, unless it is the
            // listener itself.
            (
                Process {
                    pid: 2210,
                    parent: Parent::Unknown,
                },
                1487,
                None,
            ),
            (
                Process {
                    pid: 1487,
                    parent: Parent::Unknown,
                },
                1487,
                Some(true),
            ),
            // A grandchild does not count.
            (
                Process {
                    pid: 1600,
                    parent: Parent::Of(1491),
                },
                1487,
                Some(false),
            ),
        ];
        for (process, listener, expected) in cases {
            assert_eq!(
                counts(process, compositor(listener)),
                expected,
                "{process:?}"
            );
        }
    }
}
