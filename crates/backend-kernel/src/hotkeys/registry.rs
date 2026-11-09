//! What the hotkey reader knows about each node in `/dev/input`, without
//! touching any: which are being read, which are ours, which cannot be
//! opened and whether that will change.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};

use auto_clicker_core::hotkeys::{DeviceId, HotkeyStatus, Unavailable};

/// Why a node is being opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cause {
    /// The first look at the directory, or a rescan after events were lost.
    Scan,
    /// The node appeared.
    Created,
    /// The node's attributes changed: udev set its owner or mode.
    Attrib,
}

/// What opening a node found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Opened {
    /// A keyboard, now read under this id.
    Keyboard(DeviceId),
    /// One of this program's own virtual devices.
    Ours,
    /// An input device without a bound key or a modifier key.
    NotKeyboard,
    /// The node or its device went away first.
    Gone,
    /// Permission denied. `udev_done` when udev has finished setting the
    /// node up, so its permissions are final.
    Denied { udev_done: bool },
    /// Anything else; retried when the node's attributes change.
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Reading(DeviceId),
    Ours,
    NotKeyboard,
    /// `permanent` once udev is done with the node and it still cannot be
    /// opened.
    Denied {
        permanent: bool,
    },
    Failed,
}

#[derive(Debug, Default)]
pub(crate) struct Registry {
    nodes: HashMap<OsString, State>,
}

/// Whether `name` is an evdev node, `event` and digits.
pub(crate) fn is_event_node(name: &OsStr) -> bool {
    name.to_str()
        .and_then(|name| name.strip_prefix("event"))
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

impl Registry {
    /// Whether the node should be opened now. A node already read, ours or
    /// known to have no keys we want is left alone: reading one node twice
    /// would fire every hotkey twice.
    pub(crate) fn wants_open(&self, name: &OsStr) -> bool {
        is_event_node(name)
            && match self.nodes.get(name) {
                None | Some(State::Denied { .. } | State::Failed) => true,
                Some(State::Reading(_) | State::Ours | State::NotKeyboard) => false,
            }
    }

    pub(crate) fn record(&mut self, name: &OsStr, opened: Opened, cause: Cause) {
        let state = match opened {
            Opened::Keyboard(id) => State::Reading(id),
            Opened::Ours => State::Ours,
            Opened::NotKeyboard => State::NotKeyboard,
            Opened::Gone => {
                self.nodes.remove(name);
                return;
            }
            Opened::Denied { udev_done } => State::Denied {
                permanent: udev_done || cause == Cause::Attrib,
            },
            Opened::Failed => State::Failed,
        };
        self.nodes.insert(name.to_owned(), state);
    }

    /// The node was removed; returns the device read from it, if any.
    pub(crate) fn deleted(&mut self, name: &OsStr) -> Option<DeviceId> {
        match self.nodes.remove(name) {
            Some(State::Reading(id)) => Some(id),
            _ => None,
        }
    }

    /// Reading from the node failed. A gone device's node is forgotten; any
    /// other failure is retried when the node's attributes change.
    pub(crate) fn read_failed(&mut self, name: &OsStr, gone: bool) {
        if gone {
            self.nodes.remove(name);
        } else {
            self.nodes.insert(name.to_owned(), State::Failed);
        }
    }

    pub(crate) fn status(&self) -> HotkeyStatus {
        let states = || self.nodes.values();
        if states().any(|state| matches!(state, State::Reading(_))) {
            HotkeyStatus::Available
        } else if states().any(|state| *state == State::Denied { permanent: true }) {
            HotkeyStatus::Unavailable(Unavailable::NoPermission)
        } else {
            HotkeyStatus::Unavailable(Unavailable::NoKeyboard)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> &OsStr {
        OsStr::new(text)
    }

    const ID: DeviceId = DeviceId(7);

    fn unavailable(why: Unavailable) -> HotkeyStatus {
        HotkeyStatus::Unavailable(why)
    }

    #[test]
    fn only_event_nodes_are_opened() {
        let registry = Registry::default();
        for node in ["event0", "event12"] {
            assert!(registry.wants_open(name(node)), "{node}");
        }
        for node in [
            "mice", "mouse0", "event", "event1a", "by-id", "js0", "eventx",
        ] {
            assert!(!registry.wants_open(name(node)), "{node}");
        }
    }

    #[test]
    fn a_node_is_opened_once_until_it_goes() {
        let mut registry = Registry::default();
        registry.record(name("event3"), Opened::Keyboard(ID), Cause::Created);
        let on_attrib = registry.wants_open(name("event3"));
        let deleted = registry.deleted(name("event3"));
        let on_create = registry.wants_open(name("event3"));
        let deleted_again = registry.deleted(name("event3"));
        assert_eq!(
            (on_attrib, deleted, on_create, deleted_again),
            (false, Some(ID), true, None)
        );
    }

    #[test]
    fn ours_and_keyless_nodes_are_not_reopened() {
        let mut registry = Registry::default();
        registry.record(name("event1"), Opened::Ours, Cause::Scan);
        registry.record(name("event2"), Opened::NotKeyboard, Cause::Scan);
        let reopened = [name("event1"), name("event2")].map(|node| registry.wants_open(node));
        let deleted = registry.deleted(name("event1"));
        let after_delete = registry.wants_open(name("event1"));
        assert_eq!(
            (reopened, deleted, after_delete),
            ([false, false], None, true)
        );
    }

    #[test]
    fn denied_and_failed_nodes_are_retried() {
        let mut registry = Registry::default();
        registry.record(
            name("event1"),
            Opened::Denied { udev_done: true },
            Cause::Scan,
        );
        registry.record(name("event2"), Opened::Failed, Cause::Scan);
        assert!(registry.wants_open(name("event1")));
        assert!(registry.wants_open(name("event2")));
    }

    #[test]
    fn a_gone_node_is_forgotten() {
        let mut registry = Registry::default();
        registry.record(name("event1"), Opened::Failed, Cause::Scan);
        registry.record(name("event1"), Opened::Gone, Cause::Attrib);
        assert_eq!(registry.status(), unavailable(Unavailable::NoKeyboard));
        assert!(registry.wants_open(name("event1")));
    }

    #[test]
    fn denial_is_final_once_udev_is_done_or_after_an_attrib_change() {
        let denied = |udev_done, cause| {
            let mut registry = Registry::default();
            registry.record(name("event1"), Opened::Denied { udev_done }, cause);
            registry.status()
        };
        let final_ = unavailable(Unavailable::NoPermission);
        let not_yet = unavailable(Unavailable::NoKeyboard);
        let cases = [
            (true, Cause::Scan, &final_),
            (true, Cause::Created, &final_),
            (true, Cause::Attrib, &final_),
            (false, Cause::Attrib, &final_),
            (false, Cause::Created, &not_yet),
            (false, Cause::Scan, &not_yet),
        ];
        for (udev_done, cause, expected) in cases {
            let status = denied(udev_done, cause);
            assert_eq!(&status, expected, "udev done {udev_done}, {cause:?}");
        }
    }

    #[test]
    fn one_keyboard_makes_hotkeys_available() {
        let mut registry = Registry::default();
        assert_eq!(registry.status(), unavailable(Unavailable::NoKeyboard));
        registry.record(
            name("event1"),
            Opened::Denied { udev_done: true },
            Cause::Scan,
        );
        registry.record(name("event2"), Opened::Ours, Cause::Scan);
        registry.record(name("event3"), Opened::Keyboard(ID), Cause::Scan);
        assert_eq!(registry.status(), HotkeyStatus::Available);
        registry.deleted(name("event3"));
        assert_eq!(registry.status(), unavailable(Unavailable::NoPermission));
    }

    #[test]
    fn a_failed_read_keeps_the_node_only_if_it_is_still_there() {
        let mut registry = Registry::default();
        registry.record(name("event1"), Opened::Keyboard(ID), Cause::Scan);
        registry.record(name("event2"), Opened::Keyboard(DeviceId(8)), Cause::Scan);
        registry.read_failed(name("event1"), true);
        registry.read_failed(name("event2"), false);
        assert_eq!(registry.status(), unavailable(Unavailable::NoKeyboard));
        // event1 is forgotten; event2 is kept, retried and not read.
        let retried = registry.wants_open(name("event2"));
        let read = [name("event1"), name("event2")].map(|node| registry.deleted(node));
        assert_eq!((retried, read), (true, [None, None]));
    }
}
