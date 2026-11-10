//! What the hotkey reader knows about each node in `/dev/input`, without
//! touching any: which are being read, which are ours, which cannot be
//! opened and whether that will change.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};

use auto_clicker_core::hotkeys::{DeviceId, HotkeyStatus, Unavailable};

/// What opening a node found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Opened {
    /// A keyboard, now read under this id.
    Keyboard(DeviceId),
    /// One of this program's own virtual devices.
    Ours,
    /// An input device without a bound key or a modifier key.
    NotKeyboard,
    /// The node or its device went away first.
    Gone,
    /// Permission denied. Retried when the node's attributes change, as
    /// udev sets them up after the node appears.
    Denied,
    /// Anything else, and why; retried when the node's attributes change.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Reading(DeviceId),
    Ours,
    NotKeyboard,
    Denied,
    Failed(String),
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
                None | Some(State::Denied | State::Failed(_)) => true,
                Some(State::Reading(_) | State::Ours | State::NotKeyboard) => false,
            }
    }

    pub(crate) fn record(&mut self, name: &OsStr, opened: Opened) {
        let state = match opened {
            Opened::Keyboard(id) => State::Reading(id),
            Opened::Ours => State::Ours,
            Opened::NotKeyboard => State::NotKeyboard,
            Opened::Gone => {
                self.nodes.remove(name);
                return;
            }
            Opened::Denied => State::Denied,
            Opened::Failed(why) => State::Failed(why),
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

    /// Reading from the node failed: its device is gone (`None`), and the
    /// node is forgotten, or it failed for this reason, and is retried when
    /// its attributes change.
    pub(crate) fn read_failed(&mut self, name: &OsStr, failure: Option<String>) {
        match failure {
            None => self.nodes.remove(name),
            Some(why) => self.nodes.insert(name.to_owned(), State::Failed(why)),
        };
    }

    /// Forgets every node not being read, before a rescan after lost
    /// events: any of them may have been replaced unseen. A node being read
    /// that went away ends its stream, which forgets it too.
    pub(crate) fn forget_unread(&mut self) {
        self.nodes
            .retain(|_, state| matches!(state, State::Reading(_)));
    }

    pub(crate) fn status(&self) -> HotkeyStatus {
        let states = || self.nodes.values();
        if states().any(|state| matches!(state, State::Reading(_))) {
            return HotkeyStatus::Available;
        }
        let why = if states().any(|state| *state == State::Denied) {
            Unavailable::NoPermission
        } else if let Some(failure) = self.first_failure() {
            Unavailable::ReadFailed(failure.to_owned())
        } else {
            Unavailable::NoKeyboard
        };
        HotkeyStatus::Unavailable(why)
    }

    /// The failure of the first failed node by name, so the status does not
    /// change with the map's order.
    fn first_failure(&self) -> Option<&str> {
        self.nodes
            .iter()
            .filter_map(|(name, state)| match state {
                State::Failed(why) => Some((name, why.as_str())),
                _ => None,
            })
            .min()
            .map(|(_, why)| why)
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
        registry.record(name("event3"), Opened::Keyboard(ID));
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
        registry.record(name("event1"), Opened::Ours);
        registry.record(name("event2"), Opened::NotKeyboard);
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
        registry.record(name("event1"), Opened::Denied);
        registry.record(name("event2"), Opened::Failed("EIO".into()));
        assert!(registry.wants_open(name("event1")));
        assert!(registry.wants_open(name("event2")));
    }

    #[test]
    fn a_gone_node_is_forgotten() {
        let mut registry = Registry::default();
        registry.record(name("event1"), Opened::Failed("EIO".into()));
        registry.record(name("event1"), Opened::Gone);
        assert_eq!(registry.status(), unavailable(Unavailable::NoKeyboard));
        assert!(registry.wants_open(name("event1")));
    }

    #[test]
    fn a_denied_node_means_no_permission() {
        let mut registry = Registry::default();
        registry.record(name("event1"), Opened::Denied);
        assert_eq!(registry.status(), unavailable(Unavailable::NoPermission));
    }

    #[test]
    fn one_keyboard_makes_hotkeys_available() {
        let mut registry = Registry::default();
        assert_eq!(registry.status(), unavailable(Unavailable::NoKeyboard));
        registry.record(name("event1"), Opened::Denied);
        registry.record(name("event2"), Opened::Ours);
        registry.record(name("event3"), Opened::Keyboard(ID));
        assert_eq!(registry.status(), HotkeyStatus::Available);
        registry.deleted(name("event3"));
        assert_eq!(registry.status(), unavailable(Unavailable::NoPermission));
    }

    #[test]
    fn a_failed_read_keeps_the_node_and_why_only_if_it_is_still_there() {
        let mut registry = Registry::default();
        registry.record(name("event1"), Opened::Keyboard(ID));
        registry.record(name("event2"), Opened::Keyboard(DeviceId(8)));
        registry.read_failed(name("event1"), None);
        registry.read_failed(name("event2"), Some("I/O error".into()));
        let failed = unavailable(Unavailable::ReadFailed("I/O error".into()));
        assert_eq!(registry.status(), failed);
        // event2 is kept and retried; event1 is forgotten.
        let retried = registry.wants_open(name("event2"));
        registry.read_failed(name("event2"), None);
        assert_eq!(
            (retried, registry.status()),
            (true, unavailable(Unavailable::NoKeyboard))
        );
    }

    #[test]
    fn the_first_failure_by_name_is_reported() {
        let mut registry = Registry::default();
        for (node, why) in [("event9", "nine"), ("event1", "one"), ("event5", "five")] {
            registry.record(name(node), Opened::Failed(why.into()));
        }
        let first = unavailable(Unavailable::ReadFailed("one".into()));
        assert_eq!(registry.status(), first);
    }

    #[test]
    fn permission_is_reported_before_a_failure() {
        let mut registry = Registry::default();
        registry.record(name("event1"), Opened::Failed("EIO".into()));
        registry.record(name("event2"), Opened::Denied);
        assert_eq!(registry.status(), unavailable(Unavailable::NoPermission));
    }

    #[test]
    fn a_rescan_forgets_every_node_not_read() {
        let mut registry = Registry::default();
        let unread = [
            Opened::Ours,
            Opened::NotKeyboard,
            Opened::Denied,
            Opened::Failed("EIO".into()),
        ];
        for (index, opened) in unread.into_iter().enumerate() {
            registry.record(name(&format!("event{index}")), opened);
        }
        registry.record(name("event9"), Opened::Keyboard(ID));
        registry.forget_unread();
        let reopened = ["event0", "event1", "event2", "event3", "event9"]
            .map(|node| registry.wants_open(name(node)));
        assert_eq!(reopened, [true, true, true, true, false]);
    }
}
