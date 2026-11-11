//! Playing positions: which events a recording with positions sends, in
//! which order, worked out once when it is loaded.
//!
//! - A relative move is covered when a position follows it within
//!   [`COVER_US`]; a covered move is not sent, since the position puts the
//!   pointer where the move took it. Every other move is sent (a locked
//!   pointer, a gap in the reports, the pointer pushed against an edge).
//! - A press, a release or a wheel turn is never played between a covered
//!   move and the position covering it. The position reaches the program a
//!   little after the move that caused it, so when it follows such an event
//!   within [`DELIVERY_DELAY_US`], it is sent first, at that event's time.
//!   A position later than that belongs to motion after the event and stays
//!   where it is; the move before the event is then sent as it is.
//! - A move at the same microsecond as a position before it is covered by
//!   that position too: at one time the order of a move and its report
//!   says nothing, since they come by different paths.
//!
//! The plan holds only event indices: a recording's events are not copied.

use crate::recording::{Change, Event, Layout, Output, Recording};

/// How soon after a relative move a position must follow to cover it.
pub const COVER_US: u64 = 5_000;

/// How long after a press, release or wheel turn a position may come and
/// still belong to the motion before it: the longest delivery from a move
/// to its report measured (1.2 ms, decision 0001), with a margin.
pub const DELIVERY_DELAY_US: u64 = 2_000;

/// A recording's positions, planned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Plan {
    /// The indices of the events to send, in the order to send them.
    order: Vec<u32>,
    /// The positions sent before an earlier event, ascending.
    hoisted: Vec<u32>,
    /// Every output of the recording's layout a position lies on.
    used: Vec<Output>,
}

impl Plan {
    /// The plan for `recording`; `None` when it has no positions, or more
    /// events than an index holds (no file this program reads has that
    /// many).
    pub(super) fn new(recording: &Recording) -> Option<Plan> {
        let layout = recording.layout()?;
        let events = recording.events();
        let count = u32::try_from(events.len()).ok()?;
        let (mut covered, hoists) = cover(events, count);
        cover_same_time(events, &mut covered);
        let mut hoisted: Vec<u32> = hoists.iter().map(|&(_, position)| position).collect();
        hoisted.sort_unstable();
        let mut hoists = hoists.into_iter().peekable();
        let mut order = Vec::with_capacity(events.len());
        for (index, &skip) in (0u32..).zip(&covered) {
            while let Some((_, position)) = hoists.next_if(|&(before, _)| before == index) {
                order.push(position);
            }
            if !skip && hoisted.binary_search(&index).is_err() {
                order.push(index);
            }
        }
        Some(Plan {
            order,
            hoisted,
            used: used_outputs(layout, events),
        })
    }

    /// Whether positions can be played on `current`: every output they lie
    /// on is there with the same place and size.
    pub(super) fn fits(&self, current: &Layout) -> bool {
        current.keeps(&self.used)
    }

    /// The events to send, each with the time to send it at.
    pub(super) fn steps<'a>(&'a self, events: &'a [Event]) -> impl Iterator<Item = Event> + 'a {
        let event = |index: u32| usize::try_from(index).ok().and_then(|at| events.get(at));
        self.order
            .iter()
            .enumerate()
            .filter_map(move |(step, &index)| {
                let own = *event(index)?;
                if self.hoisted.binary_search(&index).is_err() {
                    return Some(own);
                }
                // A hoisted position goes at the time of the event after it.
                let next = self.order.get(step.saturating_add(1)).copied();
                let at = next.and_then(event).map_or(own.at, |after| after.at);
                Some(Event { at, ..own })
            })
    }
}

/// What becomes of one relative move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cover {
    /// Sent as it is.
    Sent,
    /// Covered by the next position, with nothing between them.
    Covered,
    /// Covered by `position`, which goes before the event `before`.
    Hoisted { before: u32, position: u32 },
}

/// An event seen walking back: its index and time.
type Seen = Option<(u32, u64)>;

/// What becomes of a move at `at`, given the next position after it, and
/// the first press, release or wheel turn between the two.
fn cover_move(at: u64, position: Seen, first_event: Seen) -> Cover {
    let Some((position, position_at)) = position else {
        return Cover::Sent;
    };
    if position_at.saturating_sub(at) > COVER_US {
        return Cover::Sent;
    }
    match first_event {
        None => Cover::Covered,
        Some((before, event_at)) if position_at.saturating_sub(event_at) <= DELIVERY_DELAY_US => {
            Cover::Hoisted { before, position }
        }
        Some(_) => Cover::Sent,
    }
}

/// Which moves are covered, and which positions go before which event
/// (`(event, position)`, ascending by event), found walking back from the
/// end. `count` is how many events there are.
fn cover(events: &[Event], count: u32) -> (Vec<bool>, Vec<(u32, u32)>) {
    let mut covered = vec![false; events.len()];
    let mut hoists: Vec<(u32, u32)> = Vec::new();
    // The next position, and the first press, release or wheel turn before
    // it, after the event being looked at.
    let mut position: Seen = None;
    let mut first_event: Seen = None;
    for ((index, event), skip) in (0..count).zip(events).zip(&mut covered).rev() {
        match event.change {
            Change::Position(_) => {
                position = Some((index, event.at));
                first_event = None;
            }
            Change::Press(_) | Change::Release(_) | Change::Wheel(_) => {
                first_event = Some((index, event.at));
            }
            Change::Move { .. } => match cover_move(event.at, position, first_event) {
                Cover::Sent => {}
                Cover::Covered => *skip = true,
                Cover::Hoisted { before, position } => {
                    *skip = true;
                    // A position goes once, before the earliest event that
                    // would come between it and a move it covers.
                    if hoists.last().is_some_and(|&(_, last)| last == position) {
                        hoists.pop();
                    }
                    hoists.push((before, position));
                }
            },
        }
    }
    hoists.reverse();
    (covered, hoists)
}

/// Covers each move at the time of a position before it.
fn cover_same_time(events: &[Event], covered: &mut [bool]) {
    let mut position_at = None;
    for (event, skip) in events.iter().zip(covered) {
        match event.change {
            Change::Position(_) => position_at = Some(event.at),
            Change::Move { .. } if position_at == Some(event.at) => *skip = true,
            _ => {}
        }
    }
}

/// The outputs of `layout` the positions in `events` lie on.
fn used_outputs(layout: &Layout, events: &[Event]) -> Vec<Output> {
    let outputs = layout.outputs();
    let mut used = vec![false; outputs.len()];
    for event in events {
        if let Change::Position(point) = event.change {
            for (flag, output) in used.iter_mut().zip(outputs) {
                *flag |= output.contains(point);
            }
        }
    }
    outputs
        .iter()
        .zip(used)
        .filter_map(|(output, used)| used.then_some(*output))
        .collect()
}
