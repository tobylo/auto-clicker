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
//! - The first position, when no motion stamped before it was read, is
//!   where the pointer was since the start, so it goes first, at time 0.
//! - Every event counts the bytes it takes in the file at most. Once the next
//!   one would pass the budget, the timeline is full and takes nothing more.
//!
//! The last three rules depend on which input events come before a
//! position, and an input event can be read after positions stamped later
//! than it. So a position waits ("pending") until its place among the input
//! events is final, and the rules are applied in time order. A position's
//! place is final once an input event at or after its time is written,
//! since no input event goes before that one; or once a report
//! [`SETTLE_US`] later arrives, which bounds how many wait. An input event
//! read later than that still goes in time order, but the positions around
//! it may already have been merged.

use std::collections::VecDeque;

use super::{Change, Event, Point, event_bytes};

/// A millisecond, in microseconds: the span of one kept position.
const POSITION_US: u64 = 1_000;

/// How long a position waits for input events read late, at most.
const SETTLE_US: u64 = 20_000;

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

/// What [`Timeline::settle`] applies next.
enum Due {
    Position(Event),
    /// A press, release or wheel turn.
    Boundary,
}

/// The events so far, in time order.
pub(super) struct Timeline {
    events: Vec<Event>,
    /// The time of the last input event; no input event goes before it.
    last_input_at: u64,
    /// Positions whose place among the input events is not final yet, in
    /// time order.
    pending: VecDeque<Event>,
    /// The times of presses, releases and wheel turns read but not yet
    /// applied to the positions, in order.
    boundaries: VecDeque<u64>,
    /// Everything up to this time is applied.
    settled_to: u64,
    window: Window,
    /// The earliest time of motion read, if any.
    first_motion: Option<u64>,
    positioned: bool,
    /// Bytes the events take in the file at most, pending ones included.
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
            pending: VecDeque::new(),
            boundaries: VecDeque::new(),
            settled_to: 0,
            window: Window::Closed,
            first_motion: None,
            positioned: false,
            bytes: 0,
            budget,
            full: false,
        }
    }

    /// The events written so far, without the positions still pending.
    pub(super) fn events(&self) -> &[Event] {
        &self.events
    }

    /// Every event, pending positions placed.
    pub(super) fn into_events(mut self) -> Vec<Event> {
        self.settle(u64::MAX);
        self.events
    }

    /// The time of the last input event.
    pub(super) fn last_input_at(&self) -> u64 {
        self.last_input_at
    }

    /// The time of the last event of any kind.
    pub(super) fn last_at(&self) -> u64 {
        let written = self.events.last().map_or(0, |event| event.at);
        let pending = self.pending.back().map_or(0, |event| event.at);
        written.max(pending).max(self.last_input_at)
    }

    /// Whether an event did not fit the budget; nothing is taken since.
    pub(super) fn is_full(&self) -> bool {
        self.full
    }

    /// Motion stamped `at` was read, whether or not it is written yet.
    pub(super) fn motion_read(&mut self, at: u64) {
        self.first_motion = Some(self.first_motion.map_or(at, |first| first.min(at)));
    }

    /// Removes the written event at `index` (a hotkey's modifier).
    pub(super) fn remove(&mut self, index: usize) {
        if index < self.events.len() {
            self.events.remove(index);
        }
    }

    /// Writes an input event at `at`, or at the last input event's time if
    /// that is later. The positions up to it are placed first.
    pub(super) fn input(&mut self, at: u64, change: Change) {
        let at = at.max(self.last_input_at);
        if !self.reserve(event_bytes(change)) {
            return;
        }
        self.settle(at);
        let index = self.events.partition_point(|other| other.at <= at);
        self.events.insert(index, Event { at, change });
        self.last_input_at = at;
        if matches!(change, Change::Press(_) | Change::Release(_)) {
            self.event_read(at);
        }
    }

    /// A press, release or wheel turn at `at` was read: it ends the
    /// position millisecond. A wheel turn says so when it is read, since it
    /// is written only once its motion bucket ends.
    pub(super) fn event_read(&mut self, at: u64) {
        let at = at.max(self.last_input_at);
        if at <= self.settled_to {
            self.window = Window::AfterEvent;
            return;
        }
        // A boundary already queued with the same pending positions before
        // and after it does what both would, so at most one more boundary
        // waits than positions do. The earlier time is kept.
        let after = self.pending.partition_point(|event| event.at <= at);
        let low = after
            .checked_sub(1)
            .and_then(|index| self.pending.get(index))
            .map(|event| event.at);
        let high = self.pending.get(after).map(|event| event.at);
        let same = self
            .boundaries
            .iter_mut()
            .find(|b| low.is_none_or(|low| low <= **b) && high.is_none_or(|high| **b < high));
        match same {
            Some(same) => *same = (*same).min(at),
            None => {
                let index = self.boundaries.partition_point(|&other| other <= at);
                self.boundaries.insert(index, at);
            }
        }
    }

    /// A position stamped `at`. It waits until its place is final.
    pub(super) fn position(&mut self, at: u64, point: Point) {
        let event = Event {
            at,
            change: Change::Position(point),
        };
        if !self.reserve(event_bytes(event.change)) {
            return;
        }
        if at <= self.settled_to {
            return self.place(event);
        }
        let index = self.pending.partition_point(|other| other.at <= at);
        self.pending.insert(index, event);
        self.settle(at.saturating_sub(SETTLE_US));
    }

    /// Applies the pending positions and boundaries up to `frontier`, in
    /// time order; a position goes before a boundary at its own time.
    fn settle(&mut self, frontier: u64) {
        while let Some(next) = self.next_due(frontier) {
            match next {
                Due::Position(event) => self.place(event),
                Due::Boundary => self.window = Window::AfterEvent,
            }
        }
        self.settled_to = self.settled_to.max(frontier);
    }

    /// Takes the earliest pending position or boundary at or before
    /// `frontier`; a position first when both are at one time.
    fn next_due(&mut self, frontier: u64) -> Option<Due> {
        let position = self.pending.front().map(|event| event.at);
        let boundary = self.boundaries.front().copied();
        let position_first = match (position, boundary) {
            (Some(p), Some(b)) => p <= b,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let due = position_first && position.is_some_and(|p| p <= frontier);
        if due {
            return self.pending.pop_front().map(Due::Position);
        }
        let due = !position_first && boundary.is_some_and(|b| b <= frontier);
        due.then(|| self.boundaries.pop_front())
            .flatten()
            .map(|_| Due::Boundary)
    }

    /// Places a position whose place is final, by the rules above.
    fn place(&mut self, event: Event) {
        let moved = self.first_motion.is_some_and(|first| first <= event.at);
        if !self.positioned && !moved {
            return self.first_position(event.change);
        }
        if let Window::Open { at: open, start } = self.window
            && event.at < start.saturating_add(POSITION_US)
        {
            return self.replace(open, start, event);
        }
        self.insert(event);
        self.window = match self.window {
            Window::AfterEvent => Window::Closed,
            _ => Window::Open {
                at: event.at,
                start: event.at,
            },
        };
    }

    /// The pointer has not moved since the start: its position goes first.
    fn first_position(&mut self, change: Change) {
        self.events.insert(0, Event { at: 0, change });
        self.positioned = true;
        self.window = Window::Closed;
    }

    /// Puts `event` in place of the position at `open`, in the millisecond
    /// from `start`; the bytes the old one counted are freed.
    fn replace(&mut self, open: u64, start: u64, event: Event) {
        let old = self
            .events
            .iter()
            .rposition(|old| old.at == open && matches!(old.change, Change::Position(_)));
        if let Some(index) = old {
            let removed = self.events.remove(index);
            self.bytes = self.bytes.saturating_sub(event_bytes(removed.change));
        }
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

    /// Inserts a position after every event at or before its time.
    fn insert(&mut self, event: Event) {
        let index = self.events.partition_point(|other| other.at <= event.at);
        self.events.insert(index, event);
        self.positioned = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_wait_no_more_than_positions_do() {
        let mut timeline = Timeline::new(u64::MAX);
        // Wheel turns that sum to nothing write no input event, so nothing
        // places what waits.
        for at in 1..=10_000 {
            timeline.event_read(at);
        }
        assert_eq!(timeline.boundaries.len(), 1);
        timeline.position(20_000, Point { x: 1, y: 1 });
        timeline.position(20_500, Point { x: 2, y: 1 });
        for at in 10_001..=30_000 {
            timeline.event_read(at);
        }
        assert_eq!((timeline.pending.len(), timeline.boundaries.len()), (2, 3));
        // The earliest of each stretch is kept; the one at 1 µs was applied
        // when the reports placed what came 20 ms before them.
        assert_eq!(timeline.boundaries, [10_001, 20_000, 20_500]);
    }

    #[test]
    fn a_boundary_at_a_positions_time_comes_after_it() {
        let mut timeline = Timeline::new(u64::MAX);
        timeline.position(5_000, Point { x: 1, y: 1 });
        // Read from two mice out of order: one after the position, one
        // before it.
        timeline.event_read(5_000);
        timeline.event_read(4_000);
        assert_eq!(timeline.boundaries, [4_000, 5_000]);
    }
}
