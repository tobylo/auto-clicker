//! Trimming the trip to the tray: on a stop from the tray menu, the last
//! moments of a recording are the user moving to the tray icon, clicking
//! it, moving to "Stop recording" and clicking that. None of it belongs in
//! the recording.
//!
//! The rule, over the recording's mouse-only tail (everything after its last
//! key event):
//!
//! 1. When the tray said its menu opened at most [`MENU_FRESH`] before the
//!    stop, the trip began with the last button press in the tail at or
//!    before that moment: the click that opened the menu.
//! 2. Otherwise the trip began with the second-to-last button press in the
//!    tail (the click on the icon, then the click on the menu item), or the
//!    only one.
//!
//! The trip also takes the motion leading up to its first press, back to the
//! event before that motion. Everything from there on is cut; the recording
//! ends where the trip began. A key event is never cut: a press before it
//! cannot be the trip's, since the trip has no key in it.

use super::{Change, Event};
use crate::input::Held;

/// How long before a stop the menu may have opened and still count as the
/// opening that led to the stop. Older, it may have been opened and closed
/// before, and the last opening went unreported.
pub const MENU_FRESH_US: u64 = 10_000_000;

/// Where the trip to the tray begins in `events`, for a recording ending at
/// `end` whose tray menu last opened at `menu_opened`, both in microseconds
/// since the start. `events.len()` when no trip is found.
pub fn tray_cut(events: &[Event], menu_opened: Option<u64>, end: u64) -> usize {
    let tail = events
        .iter()
        .rposition(|event| is_key(event.change))
        .map_or(0, |index| index.saturating_add(1));
    let presses: Vec<usize> = (tail..events.len())
        .filter(|&index| {
            events
                .get(index)
                .is_some_and(|event| is_button_press(event.change))
        })
        .collect();
    let opened = menu_opened.filter(|&opened| opened <= end && end - opened <= MENU_FRESH_US);
    let marked = opened.and_then(|opened| {
        presses
            .iter()
            .rev()
            .find(|&&index| events.get(index).is_some_and(|event| event.at <= opened))
    });
    let first = marked.or_else(|| presses.iter().rev().take(2).next_back());
    let Some(&first) = first else {
        return events.len();
    };
    let motion_before = events
        .get(..first)
        .unwrap_or_default()
        .iter()
        .rev()
        .take_while(|event| matches!(event.change, Change::Move { .. }))
        .count();
    first.saturating_sub(motion_before)
}

fn is_key(change: Change) -> bool {
    matches!(
        change,
        Change::Press(Held::Key(_)) | Change::Release(Held::Key(_))
    )
}

fn is_button_press(change: Change) -> bool {
    matches!(change, Change::Press(Held::Button(_)))
}
