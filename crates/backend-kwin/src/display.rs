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
//! counts, as before W01. A lookup that fails is tried again after the
//! watcher's back-off, and a KWin that still cannot be told then counts
//! too: a failed lookup never takes the layout away for KWin's life.

use std::future::ready;
use std::num::NonZeroU32;
use std::rc::Rc;

use futures_util::{StreamExt, stream};

use crate::scripting::{Owners, Scripting, bounded};
use crate::session::Session;
use crate::watcher::BACK_OFF;

/// A process as the check needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Process {
    pub(crate) pid: u32,
    /// `None` when it is gone or cannot be read.
    pub(crate) parent: Option<u32>,
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
    match bounded(session.scripting.owner_process(&owner)).await {
        Ok(process) => counts(process, compositor).then_some(owner),
        Err(err) => {
            eprintln!(
                "auto-clicker: cannot tell whether KWin runs this display, so its scripts are \
                 left to the next start: {}",
                err.0
            );
            None
        }
    }
}

/// `owners` with each checked: a new owner reads as none until it counts,
/// so nothing is loaded into a KWin before it is checked.
pub(crate) fn each_checked<S: Scripting + 'static>(
    session: Rc<Session<S>>,
    owners: Owners,
) -> Owners {
    if session.compositor.is_none() {
        return owners;
    }
    Box::pin(owners.flat_map(move |owner| {
        let session = Rc::clone(&session);
        let checked = async move { checked(&session, owner).await };
        stream::once(ready(None)).chain(stream::once(checked))
    }))
}

/// Whether `owner`'s process is `compositor` or its child.
async fn on_display<S: Scripting>(scripting: &S, owner: &str, compositor: NonZeroU32) -> bool {
    let waits = std::iter::once(None).chain(BACK_OFF.iter().copied().map(Some));
    for wait in waits {
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
        }
        match bounded(scripting.owner_process(owner)).await {
            Ok(process) => return counts(process, compositor),
            Err(err) => eprintln!(
                "auto-clicker: cannot tell whether KWin runs this display: {}",
                err.0
            ),
        }
    }
    eprintln!("auto-clicker: KWin is taken to run this display, as it cannot be told");
    true
}

fn counts(process: Process, compositor: NonZeroU32) -> bool {
    let compositor = compositor.get();
    let ours = process.pid == compositor || process.parent == Some(compositor);
    if !ours {
        eprintln!(
            "auto-clicker: KWin runs another display than this daemon's, so its layout and \
             positions are not used"
        );
    }
    ours
}

/// The parent of process `pid`, from `/proc/<pid>/status`; `None` when it
/// is gone or the line does not parse.
pub(crate) fn parent_of(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    parse_parent(&status)
}

fn parse_parent(status: &str) -> Option<u32> {
    let line = status.lines().find_map(|line| line.strip_prefix("PPid:"))?;
    line.trim().parse().ok()
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
                    parent: Some(1487),
                },
                1487,
                true,
            ),
            // A KWin that listens on its socket itself.
            (
                Process {
                    pid: 1487,
                    parent: Some(1),
                },
                1487,
                true,
            ),
            // Another session's KWin, or one under a nested compositor.
            (
                Process {
                    pid: 2210,
                    parent: Some(2201),
                },
                1487,
                false,
            ),
            (
                Process {
                    pid: 2210,
                    parent: None,
                },
                1487,
                false,
            ),
            // A grandchild does not count.
            (
                Process {
                    pid: 1600,
                    parent: Some(1491),
                },
                1487,
                false,
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

    #[test]
    fn the_parent_is_read_from_the_status_file() {
        let status = "Name:\tkwin_wayland\nState:\tS (sleeping)\nPid:\t1491\nPPid:\t1487\n";
        assert_eq!(parse_parent(status), Some(1487));
        assert_eq!(parse_parent("Name:\tx\nPPid:\tx\n"), None);
        assert_eq!(parse_parent("Name:\tx\n"), None);
        assert_eq!(
            parent_of(std::process::id()),
            Some(std::os::unix::process::parent_id())
        );
    }
}
