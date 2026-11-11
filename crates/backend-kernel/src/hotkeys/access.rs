//! One look at the keyboards before any reader runs, for picking the route
//! hotkeys take at start.

use std::path::PathBuf;

use auto_clicker_core::hotkeys::{Bindings, HotkeyStatus, Unavailable};
use auto_clicker_core::recording::RecordingSlot;

use super::node::Paths;
use super::watch::Watch;
use super::{Reader, Setup, own_paths};
use crate::DeviceIdentity;

/// Whether the keyboards can be read for `bindings` now: what the input
/// reader would report right after its start, leaving out the program's
/// `own` devices.
///
/// It opens every node as the reader does and counts only a keyboard with a
/// bound key or a modifier: a readable gamepad alone is no keyboard. A node
/// it may not open makes [`Unavailable::NoPermission`]; a node that went
/// away counts for nothing. Every node it opened is closed again before it
/// returns: the reader opens its own, as it must for devices plugged in
/// later.
///
/// Runs inside a tokio runtime with I/O enabled.
pub fn keyboard_access(bindings: Bindings, own: &[DeviceIdentity]) -> HotkeyStatus {
    look(Paths::system(), bindings, own_paths(own))
}

/// [`keyboard_access`] on the nodes in `paths`. It starts a watch on the
/// directory as the reader does, so a watch the reader could not start
/// either (no inotify instance left) counts as [`Unavailable::WatchFailed`].
pub(super) fn look(paths: Paths, bindings: Bindings, own: Vec<PathBuf>) -> HotkeyStatus {
    let _watch = match Watch::new(&paths.dev) {
        Ok(watch) => watch,
        Err(err) => return HotkeyStatus::Unavailable(Unavailable::WatchFailed(err.to_string())),
    };
    // The check is for the hotkeys, so it matches them: it captures nothing.
    let setup = Setup {
        bindings,
        capture: None,
        own,
        feed: RecordingSlot::new().feed(),
    };
    let mut reader = Reader::new(paths, setup, |_| {});
    match reader.scan() {
        Ok(()) => reader.registry.status(),
        Err(err) => HotkeyStatus::Unavailable(Unavailable::WatchFailed(err.to_string())),
    }
}
