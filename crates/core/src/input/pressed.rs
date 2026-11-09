//! The set of buttons and keys the program holds down.

use std::collections::BTreeSet;

use super::{Button, Key};

/// A button or key the program has pressed and not yet released.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Held {
    Button(Button),
    Key(Key),
}

/// Ordered, so releases happen in a fixed order and "the first error" of a
/// release pass always means the same item.
#[derive(Debug, Default)]
pub(super) struct PressedSet(BTreeSet<Held>);

impl PressedSet {
    /// Records `item` as held. Holding it again changes nothing, as in the
    /// kernel, which drops a press of a key that is already down.
    pub(super) fn insert(&mut self, item: Held) {
        self.0.insert(item);
    }

    pub(super) fn remove(&mut self, item: Held) {
        self.0.remove(&item);
    }

    pub(super) fn contains(&self, item: Held) -> bool {
        self.0.contains(&item)
    }

    /// A copy of the held items, in release order.
    pub(super) fn snapshot(&self) -> Vec<Held> {
        self.0.iter().copied().collect()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = Held> + '_ {
        self.0.iter().copied()
    }
}
