//! The project page's facts (docs/facts.json) against the program itself: the
//! hotkeys it lists are the default bindings, and its fastest stable interval
//! is the interval floor. scripts/check-facts.sh checks the file against the
//! README.

// Test code: a panic is the failure report.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use auto_clicker_core::clicker::INTERVAL_FLOOR;
use auto_clicker_core::hotkeys::{Action, Binding, Bindings};
use serde_json::Value;

fn facts() -> Value {
    serde_json::from_str(include_str!("../../../docs/facts.json")).expect("docs/facts.json is JSON")
}

fn text<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("docs/facts.json has no text at {pointer}"))
}

#[test]
fn the_hotkeys_are_the_default_bindings_in_action_order() {
    let facts = facts();
    let modifiers = text(&facts, "/hotkeys/modifiers");
    let listed = facts
        .pointer("/hotkeys/bindings")
        .and_then(Value::as_array)
        .expect("docs/facts.json lists hotkeys");
    assert_eq!(listed.len(), Action::ALL.len());
    let defaults = Bindings::default();
    for (action, hotkey) in Action::ALL.into_iter().zip(listed) {
        let written = format!("{modifiers}+{}", text(hotkey, "/key"));
        let binding = Binding::parse(&written).expect("a hotkey the config would accept");
        assert_eq!(
            binding,
            defaults.binding(action),
            "{written} for {action:?}"
        );
    }
}

#[test]
fn the_fastest_stable_interval_is_the_interval_floor() {
    let facts = facts();
    let figure = facts
        .pointer("/figures")
        .and_then(Value::as_array)
        .and_then(|figures| figures.iter().find(|f| f["id"] == "fastest-interval"))
        .expect("docs/facts.json has the fastest-interval figure");
    assert_eq!(text(figure, "/unit"), "ms");
    let millis: u64 = text(figure, "/value")
        .parse()
        .expect("a whole number of ms");
    assert_eq!(Duration::from_millis(millis), INTERVAL_FLOOR);
}
