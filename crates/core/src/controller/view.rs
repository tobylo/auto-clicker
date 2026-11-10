//! What the tray shows: the controller's state as plain text.
//!
//! The controller publishes a [`View`] after every change; the tray only
//! reads it. The text is plain: a tray that renders markup escapes it.

use std::time::Duration;

use crate::clicker::{ClickerSettings, Mode, Position, Repeat};
use crate::hotkeys::Binding;
use crate::input::{Button, Key};

/// What the controller is doing, as the tray shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    /// A start from the tray counts down; whole seconds left, rounded up.
    StartingIn(u64),
    /// A start from the hotkey waits for its modifiers to come up.
    WaitingForRelease,
    /// A start waits for the input devices to open, which can mean the
    /// desktop's permission dialog.
    Opening,
    Clicking,
}

/// Everything the tray shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub activity: Activity,
    pub settings: ClickerSettings,
    /// The hotkey that starts and stops clicking.
    pub hotkey: Binding,
    /// What last happened that the user should know: why a start was
    /// refused or cancelled, or why a run stopped.
    pub notice: Option<String>,
    /// What is wrong now: an invalid config, unreadable keyboards, a locked
    /// session, devices that could not be opened.
    pub problems: Vec<String>,
}

impl View {
    /// One line on the state, for the tray's title and tooltip title.
    pub fn title(&self) -> String {
        match self.activity {
            Activity::Idle => "Auto clicker: idle".to_owned(),
            Activity::StartingIn(seconds) => format!("Auto clicker: starts in {seconds} s"),
            Activity::WaitingForRelease => {
                format!(
                    "Auto clicker: starts when you let go of {}",
                    modifiers_of(self.hotkey)
                )
            }
            Activity::Opening => {
                "Auto clicker: starting; allow input if the desktop asks".to_owned()
            }
            Activity::Clicking => "Auto clicker: clicking".to_owned(),
        }
    }

    /// The tooltip's lines: what a run does, the hotkey, the last notice and
    /// every problem.
    pub fn details(&self) -> Vec<String> {
        let mut lines = vec![
            describe(&self.settings),
            format!("{} starts and stops clicking", self.hotkey),
        ];
        lines.extend(self.notice.iter().cloned());
        lines.extend(self.problems.iter().cloned());
        lines
    }
}

/// The binding's modifiers, "Ctrl" or "Ctrl and Shift"; "the hotkey" when it
/// has none.
fn modifiers_of(binding: Binding) -> String {
    let names: Vec<&str> = binding.modifiers().iter().map(|m| m.name()).collect();
    match names.split_last() {
        None => "the hotkey".to_owned(),
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// One sentence on what a run with `settings` does.
pub fn describe(settings: &ClickerSettings) -> String {
    let button = button_name(settings.button);
    let key = key_name(settings.key);
    let action = match settings.mode {
        Mode::Click => format!("Clicks the {button} button"),
        Mode::DoubleClick => format!("Double-clicks the {button} button"),
        Mode::HoldButton => format!("Holds the {button} button down"),
        Mode::KeyRepeat => format!("Presses {key}"),
        Mode::HoldKey => format!("Holds {key} down"),
    };
    let mut text = action;
    if settings.mode.repeats() {
        text.push_str(&format!(" every {}", duration(settings.interval.get())));
        if settings.jitter.percent() > 0 {
            text.push_str(&format!(" (±{}%)", settings.jitter.percent()));
        }
        if let Repeat::Times(count) = settings.repeat {
            text.push_str(&format!(", {count} times"));
        }
    }
    let uses_button = !matches!(settings.mode, Mode::KeyRepeat | Mode::HoldKey);
    if uses_button && matches!(settings.position, Position::Fixed(_)) {
        text.push_str(" at a fixed point");
    }
    text
}

/// The button as the config file names it.
fn button_name(button: Button) -> &'static str {
    match button {
        Button::Left => "left",
        Button::Right => "right",
        Button::Middle => "middle",
        Button::Side => "back",
        Button::Extra => "forward",
    }
}

fn key_name(key: Key) -> String {
    key.name()
        .map_or_else(|| format!("key {}", key.code()), str::to_owned)
}

/// "100 ms", or "2 s" for whole seconds.
fn duration(interval: Duration) -> String {
    let millis = interval.as_millis();
    if millis >= 1000 && millis.is_multiple_of(1000) {
        format!("{} s", millis / 1000)
    } else {
        format!("{millis} ms")
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::*;
    use crate::clicker::{Interval, Jitter};
    use crate::hotkeys::Bindings;
    use crate::input::AbsolutePosition;

    fn settings(mode: Mode) -> ClickerSettings {
        ClickerSettings {
            mode,
            ..ClickerSettings::default()
        }
    }

    #[test]
    fn describes_each_mode() {
        let cases = [
            (Mode::Click, "Clicks the left button every 100 ms"),
            (
                Mode::DoubleClick,
                "Double-clicks the left button every 100 ms",
            ),
            (Mode::HoldButton, "Holds the left button down"),
            (Mode::KeyRepeat, "Presses space every 100 ms"),
            (Mode::HoldKey, "Holds space down"),
        ];
        for (mode, text) in cases {
            assert_eq!(describe(&settings(mode)), text, "{mode:?}");
        }
    }

    #[test]
    fn describes_jitter_count_position_and_whole_seconds() {
        let settings = ClickerSettings {
            interval: Interval::new(Duration::from_secs(2)).unwrap(),
            jitter: Jitter::new(10).unwrap(),
            button: Button::Side,
            repeat: Repeat::Times(NonZeroU32::new(50).unwrap()),
            position: Position::Fixed(AbsolutePosition { x: 1, y: 2 }),
            ..ClickerSettings::default()
        };
        assert_eq!(
            describe(&settings),
            "Clicks the back button every 2 s (±10%), 50 times at a fixed point"
        );
    }

    #[test]
    fn only_whole_seconds_are_shown_in_seconds() {
        let shown = |ms| duration(Duration::from_millis(ms));
        assert_eq!(shown(1500), "1500 ms");
        assert_eq!(shown(1000), "1 s");
        assert_eq!(shown(100), "100 ms");
    }

    #[test]
    fn a_key_mode_ignores_the_fixed_point_and_names_unnamed_keys_by_code() {
        let settings = ClickerSettings {
            mode: Mode::HoldKey,
            key: Key::new(248).unwrap(),
            position: Position::Fixed(AbsolutePosition { x: 1, y: 2 }),
            ..ClickerSettings::default()
        };
        assert_eq!(describe(&settings), "Holds key 248 down");
    }

    fn view(activity: Activity, hotkey: &str) -> View {
        View {
            activity,
            settings: ClickerSettings::default(),
            hotkey: Binding::parse(hotkey).unwrap(),
            notice: None,
            problems: Vec::new(),
        }
    }

    #[test]
    fn titles_name_the_state() {
        let cases = [
            (Activity::Idle, "Ctrl+F6", "Auto clicker: idle"),
            (
                Activity::StartingIn(3),
                "Ctrl+F6",
                "Auto clicker: starts in 3 s",
            ),
            (
                Activity::WaitingForRelease,
                "Ctrl+F6",
                "Auto clicker: starts when you let go of Ctrl",
            ),
            (
                Activity::WaitingForRelease,
                "Ctrl+Shift+Alt+F6",
                "Auto clicker: starts when you let go of Ctrl, Shift and Alt",
            ),
            (
                Activity::WaitingForRelease,
                "F13",
                "Auto clicker: starts when you let go of the hotkey",
            ),
            (Activity::Clicking, "Ctrl+F6", "Auto clicker: clicking"),
        ];
        for (activity, hotkey, title) in cases {
            assert_eq!(view(activity, hotkey).title(), title);
        }
    }

    #[test]
    fn details_list_settings_hotkey_notice_and_problems() {
        let mut shown = view(Activity::Idle, "Ctrl+F6");
        shown.hotkey = Bindings::default().toggle_clicking;
        shown.notice = Some("notice".to_owned());
        shown.problems = vec!["one".to_owned(), "two".to_owned()];
        assert_eq!(
            shown.details(),
            [
                "Clicks the left button every 100 ms",
                "Ctrl+F6 starts and stops clicking",
                "notice",
                "one",
                "two",
            ]
        );
    }
}
