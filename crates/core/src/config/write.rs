//! The config file as the settings window saves it: every setting written
//! out, in one order, under a header that says the window manages the file.
//!
//! Every value is a number or a word from a fixed set (modes, buttons, key
//! and modifier names, routes), so nothing written needs escaping.

use std::fmt::Write;

use super::Config;
use super::draft::{CountDraft, Draft, IntervalDraft, PositionDraft};
use super::validate::VERSION;
use crate::input::Key;
use crate::playback::Speed;

const HEADER: &str = "\
# auto-clicker settings, written by its Settings window.
# The window writes the whole file anew when it saves, without comments, so
# edit settings there. Each setting is described in the README, under
# \"Configuration\".
";

/// The file's text for `config`, which [`Config::parse`] reads back to
/// `config`.
pub fn render(config: &Config) -> String {
    let draft = Draft::from(config);
    let mut text = String::from(HEADER);
    let mut lines = Lines(&mut text);
    lines.value("version", VERSION.to_string());
    lines.value("tray_start_delay", draft.tray_start_delay.to_string());
    let clicker = &draft.clicker;
    lines.section("clicker");
    lines.word("mode", clicker.mode.word());
    match clicker.interval {
        IntervalDraft::Ms(ms) => lines.value("interval_ms", ms.to_string()),
        IntervalDraft::PerSecond(rate) => lines.value("clicks_per_second", format!("{rate:?}")),
    }
    lines.value("jitter_percent", clicker.jitter_percent.to_string());
    lines.word("button", clicker.button.word());
    lines.value("key", key(clicker.key));
    lines.value("position", position(clicker.position));
    lines.value("repeat", count(clicker.repeat));
    lines.section("hotkeys");
    lines.word("toggle_clicking", &draft.hotkeys.toggle_clicking);
    lines.word("toggle_recording", &draft.hotkeys.toggle_recording);
    lines.word("toggle_playback", &draft.hotkeys.toggle_playback);
    lines.section("input");
    lines.word("output", draft.input.output.word());
    lines.word("hotkeys", draft.input.hotkeys.word());
    lines.section("playback");
    lines.value("speed", speed(config.playback.speed));
    lines.value("repeat", count(draft.playback.repeat));
    text
}

/// Writes `key = value` lines.
struct Lines<'a>(&'a mut String);

impl Lines<'_> {
    fn value(&mut self, key: &str, value: String) {
        // Writing to a String cannot fail.
        let _ = writeln!(self.0, "{key} = {value}");
    }

    fn section(&mut self, name: &str) {
        let _ = write!(self.0, "\n[{name}]\n");
    }

    /// A quoted word: a name from a fixed set, which needs no escaping.
    fn word(&mut self, key: &str, word: &str) {
        self.value(key, format!("\"{word}\""));
    }
}

/// A key by name, or by code when it has none.
fn key(code: u16) -> String {
    match Key::new(code).and_then(Key::name) {
        Some(name) => format!("\"{name}\""),
        None => code.to_string(),
    }
}

fn position(position: PositionDraft) -> String {
    match position {
        PositionDraft::Cursor => "\"cursor\"".to_owned(),
        PositionDraft::Fixed { x, y } => format!("{{ x = {x}, y = {y} }}"),
    }
}

fn count(count: CountDraft) -> String {
    match count {
        CountDraft::UntilStopped => "\"until_stopped\"".to_owned(),
        CountDraft::Times(times) => times.to_string(),
    }
}

/// The speed as a decimal with a point: "1.0", "1.5", "0.25".
fn speed(speed: Speed) -> String {
    let shown = speed.to_string();
    if shown.contains('.') {
        shown
    } else {
        format!("{shown}.0")
    }
}
