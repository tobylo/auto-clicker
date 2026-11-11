//! The events of a recording in progress, in time order, and the rules for
//! where each goes.
//!
//! - Input events (keys, buttons, motion, wheel) keep the order they were
//!   read in. One read late is moved forward to the last input event's time
//!   rather than before it, since putting a key's release before its press
//!   would make the recording invalid.
//! - Positions go by their own time, among everything. So an input event
//!   read late still goes before the positions stamped after it.
//! - At most one position per millisecond is kept, the latest. A press, a
//!   release or a wheel turn ends that millisecond, and the first position
//!   after it is kept as it is: it may belong to the move before the event.
//! - The first position, when no motion came before it, is where the
//!   pointer was since the start, so it goes first, at time 0.
//! - Every event counts the bytes it takes in the file at most. Once the next
//!   one would pass the budget, the timeline is full and takes nothing more.

use super::{Change, Event, Point, event_bytes};

/// A millisecond, in microseconds: the span of one kept position.
const POSITION_US: u64 = 1_000;

/// Which position the next one may replace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Window {
    /// None: the next position opens a millisecond of its own.
    Closed,
    /// A press, release or wheel turn came last: the next position is kept
    /// as it is.
    AfterEvent,
    /// The position at `at` may be replaced until `start` plus a
    /// millisecond.
    Open { at: u64, start: u64 },
}

/// The events so far, in time order.
pub(super) struct Timeline {
    events: Vec<Event>,
    /// The time of the last input event; no input event goes before it.
    last_input_at: u64,
    window: Window,
    /// Motion was seen, so a first position no longer stands for the start.
    moved: bool,
    positioned: bool,
    /// Bytes the events take in the file at most.
    bytes: u64,
    budget: u64,
    full: bool,
}

impl Timeline {
    /// An empty timeline whose events may take `budget` bytes in a file.
    pub(super) fn new(budget: u64) -> Timeline {
        Timeline {
            events: Vec::new(),
            last_input_at: 0,
            window: Window::Closed,
            moved: false,
            positioned: false,
            bytes: 0,
            budget,
            full: false,
        }
    }

    pub(super) fn events(&self) -> &[Event] {
        &self.events
    }

    pub(super) fn into_events(self) -> Vec<Event> {
        self.events
    }

    /// The time of the last input event.
    pub(super) fn last_input_at(&self) -> u64 {
        self.last_input_at
    }

    /// The time of the last event of any kind.
    pub(super) fn last_at(&self) -> u64 {
        let last = self.events.last().map_or(0, |event| event.at);
        last.max(self.last_input_at)
    }

    /// Whether an event did not fit the budget; nothing is taken since.
    pub(super) fn is_full(&self) -> bool {
        self.full
    }

    /// Motion was read, whether or not it is written yet.
    pub(super) fn motion_seen(&mut self) {
        self.moved = true;
    }

    /// Removes the event at `index` (a hotkey's modifier).
    pub(super) fn remove(&mut self, index: usize) {
        if index < self.events.len() {
            self.events.remove(index);
        }
    }

    /// Writes an input event at `at`, or at the last input event's time if
    /// that is later.
    pub(super) fn input(&mut self, at: u64, change: Change) {
        let at = at.max(self.last_input_at);
        if !self.reserve(event_bytes(change)) {
            return;
        }
        self.insert(Event { at, change });
        self.last_input_at = at;
        if matches!(
            change,
            Change::Press(_) | Change::Release(_) | Change::Wheel(_)
        ) {
            self.window = match self.window {
                // The open position is later: it is the first after this.
                Window::Open { at: open, .. } if open > at => Window::Closed,
                _ => Window::AfterEvent,
            };
        }
    }

    /// Writes a position stamped `at`.
    pub(super) fn position(&mut self, at: u64, point: Point) {
        let change = Change::Position(point);
        if !self.positioned && !self.moved {
            return self.first_position(change);
        }
        if let Window::Open { at: open, start } = self.window
            && at < start.saturating_add(POSITION_US)
        {
            return self.replace(open, start, Event { at, change });
        }
        if !self.reserve(event_bytes(change)) {
            return;
        }
        self.insert(Event { at, change });
        self.window = match self.window {
            Window::AfterEvent => Window::Closed,
            _ => Window::Open { at, start: at },
        };
    }

    /// The pointer has not moved since the start: its position goes first.
    fn first_position(&mut self, change: Change) {
        if !self.reserve(event_bytes(change)) {
            return;
        }
        self.events.insert(0, Event { at: 0, change });
        self.positioned = true;
        self.window = Window::Closed;
    }

    /// Puts `event` in place of the position at `open`, in the millisecond
    /// from `start`.
    fn replace(&mut self, open: u64, start: u64, event: Event) {
        let old = self
            .events
            .iter()
            .rposition(|old| old.at == open && matches!(old.change, Change::Position(_)));
        let Some(index) = old else {
            return;
        };
        let freed = self
            .events
            .get(index)
            .map_or(0, |old| event_bytes(old.change));
        let kept = self.bytes;
        self.bytes = self.bytes.saturating_sub(freed);
        if !self.reserve(event_bytes(event.change)) {
            // The old position stays, counted as before.
            self.bytes = kept;
            return;
        }
        self.events.remove(index);
        self.insert(event);
        self.window = Window::Open {
            at: event.at,
            start,
        };
    }

    /// Counts `bytes` against the budget; `false`, and full from then on,
    /// when they do not fit.
    fn reserve(&mut self, bytes: u64) -> bool {
        let total = self.bytes.saturating_add(bytes);
        if self.full || total > self.budget {
            self.full = true;
            return false;
        }
        self.bytes = total;
        true
    }

    /// Inserts `event` after every event at or before its time.
    fn insert(&mut self, event: Event) {
        let index = self.events.partition_point(|other| other.at <= event.at);
        self.events.insert(index, event);
        if matches!(event.change, Change::Position(_)) {
            self.positioned = true;
        }
    }
}
