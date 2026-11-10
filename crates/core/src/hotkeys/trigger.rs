//! Hotkeys the desktop delivers itself: the triggers it reports, and which
//! runs they rule out.
//!
//! A desktop that delivers hotkeys (KDE's GlobalShortcuts portal) says which
//! shortcut fired, never which device the keys came from. So the program
//! cannot tell its own key output from the user's, and stays clear of it
//! instead: a run that would press a hotkey's key is refused.
//!
//! The desktop reports each trigger as text for people, in the user's
//! language ("Strg+Umschalt+F6"), and names keys by what the layout prints
//! on them. Only one part of that text means the same key everywhere: a
//! function key, F1 to F24. So a trigger is understood when it ends in one,
//! and counts as unreadable otherwise.

use std::fmt;
use std::time::Duration;

use super::Action;
use crate::input::Key;

/// How long after the desktop reports a hotkey released a run it started
/// begins. The desktop reports the release at the first key lifted, usually
/// the function key while Ctrl and Shift are still down, and never says when
/// those come up; the pause keeps the first clicks from being Ctrl+clicks.
pub const DESKTOP_RELEASE_PAUSE: Duration = Duration::from_secs(1);

/// The longest trigger text kept, in characters.
const MAX_TEXT: usize = 80;

/// The trigger of one action, as the desktop reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    /// The desktop's text, for showing; `None` until it reported one.
    text: Option<String>,
    /// The keys that set it off; `None` when they cannot be told.
    keys: Option<Vec<Key>>,
}

impl Trigger {
    /// Not reported yet: nothing to show, and its keys unknown.
    pub const UNKNOWN: Trigger = Trigger {
        text: None,
        keys: None,
    };

    /// The trigger the desktop describes as `description`. Empty means the
    /// action has no trigger.
    pub fn reported(description: &str) -> Trigger {
        let text: String = description
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_TEXT)
            .collect();
        let text = text.trim().to_owned();
        // Read from the kept text, so what is shown is what was checked.
        let keys = keys_of(&text);
        Trigger {
            text: Some(text),
            keys,
        }
    }

    /// The desktop's text: `None` until it reported one, empty when the
    /// action has no trigger.
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }
}

/// The key of every trigger in `text` ("Ctrl+F6, Meta+F7"), or `None` when
/// one of them does not end in a function key.
fn keys_of(text: &str) -> Option<Vec<Key>> {
    if text.is_empty() {
        return Some(Vec::new());
    }
    text.split(',').map(function_key).collect()
}

/// The function key a trigger such as "Ctrl+Shift+F6" ends in.
fn function_key(trigger: &str) -> Option<Key> {
    let word = trigger.trim().rsplit('+').next()?;
    let number: u8 = word.strip_prefix('F')?.parse().ok()?;
    // "F06" and "F+6" are not how a desktop writes a key.
    let plain = word.len() == 1 + number.to_string().len();
    (plain && (1..=24).contains(&number))
        .then(|| Key::from_name(&format!("f{number}")))
        .flatten()
}

/// The trigger of every action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Triggers {
    toggle_clicking: Trigger,
    toggle_recording: Trigger,
    toggle_playback: Trigger,
}

impl Triggers {
    /// Before the desktop reported any.
    pub const fn unknown() -> Triggers {
        Triggers {
            toggle_clicking: Trigger::UNKNOWN,
            toggle_recording: Trigger::UNKNOWN,
            toggle_playback: Trigger::UNKNOWN,
        }
    }

    pub fn of(&self, action: Action) -> &Trigger {
        match action {
            Action::ToggleClicking => &self.toggle_clicking,
            Action::ToggleRecording => &self.toggle_recording,
            Action::TogglePlayback => &self.toggle_playback,
        }
    }

    pub fn set(&mut self, action: Action, trigger: Trigger) {
        match action {
            Action::ToggleClicking => self.toggle_clicking = trigger,
            Action::ToggleRecording => self.toggle_recording = trigger,
            Action::TogglePlayback => self.toggle_playback = trigger,
        }
    }

    /// Why a run may not press `key`, if it may not: the key could set off a
    /// hotkey, or change which hotkey the user's own press sets off.
    pub fn refusal(&self, key: Key) -> Option<Refusal> {
        if super::Modifier::of_code(key.code()).is_some() {
            return Some(Refusal::Modifier(key));
        }
        Action::ALL
            .into_iter()
            .find_map(|action| self.of(action).refusal(key))
    }
}

impl Trigger {
    fn refusal(&self, key: Key) -> Option<Refusal> {
        match (&self.keys, &self.text) {
            (Some(keys), _) if !keys.contains(&key) => None,
            (Some(_), text) => Some(Refusal::HotkeyKey {
                key,
                trigger: text.clone().unwrap_or_default(),
            }),
            (None, Some(text)) => Some(Refusal::Unreadable(text.clone())),
            (None, None) => Some(Refusal::NotReported),
        }
    }
}

/// Why a run that presses a key may not start while the desktop delivers
/// the hotkeys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The key is a modifier: held by the program, it would turn the user's
    /// Ctrl+F6 into Ctrl+Shift+F6.
    Modifier(Key),
    /// The key is the key of this trigger.
    HotkeyKey { key: Key, trigger: String },
    /// This trigger does not end in a function key, so its key is unknown.
    Unreadable(String),
    /// The desktop has not reported the triggers yet.
    NotReported,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Modifier(key) => write!(
                f,
                "the clicker's key, {}, is a modifier, and holding it would change which hotkey \
                 the desktop sees; choose another key",
                key_name(*key)
            ),
            Refusal::HotkeyKey { key, trigger } => write!(
                f,
                "the clicker's key, {}, is the key of the hotkey {trigger}, and the desktop \
                 cannot tell the program's key presses from yours; change the key or the hotkey",
                key_name(*key)
            ),
            Refusal::Unreadable(trigger) => write!(
                f,
                "the clicker presses a key, and the hotkey \u{201c}{trigger}\u{201d} is not one \
                 the program can check it against; give every hotkey a function key (F1 to F24) \
                 in System Settings"
            ),
            Refusal::NotReported => f.write_str(
                "the clicker presses a key, and the desktop has not said yet which keys the \
                 hotkeys use",
            ),
        }
    }
}

fn key_name(key: Key) -> String {
    key.name()
        .map_or_else(|| format!("key {}", key.code()), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> Key {
        Key::from_name(name).unwrap()
    }

    /// Triggers as the desktop would report them for the three actions.
    fn triggers(clicking: &str, recording: &str, playback: &str) -> Triggers {
        let mut triggers = Triggers::unknown();
        triggers.set(Action::ToggleClicking, Trigger::reported(clicking));
        triggers.set(Action::ToggleRecording, Trigger::reported(recording));
        triggers.set(Action::TogglePlayback, Trigger::reported(playback));
        triggers
    }

    #[test]
    fn a_trigger_is_read_by_its_function_key_whatever_the_modifiers_are_called() {
        let cases: [(&str, Option<&[&str]>); 14] = [
            ("Ctrl+Shift+F6", Some(&["f6"])),
            ("Strg+Umschalt+F6", Some(&["f6"])),
            ("F24", Some(&["f24"])),
            ("Meta+F1", Some(&["f1"])),
            ("Ctrl+F6, Meta+F7", Some(&["f6", "f7"])),
            ("", Some(&[])),
            ("Ctrl+Space", None),
            ("Ctrl+Z", None),
            ("Ctrl+F25", None),
            ("Ctrl+F0", None),
            ("Ctrl+F06", None),
            ("Ctrl+F+6", None),
            ("Ctrl+F6, Ctrl+K", None),
            ("Ctrl+,", None),
        ];
        for (text, keys) in cases {
            let expected: Option<Vec<Key>> =
                keys.map(|names| names.iter().map(|n| key(n)).collect());
            assert_eq!(Trigger::reported(text).keys, expected, "{text:?}");
        }
    }

    #[test]
    fn the_text_is_kept_for_showing_without_control_characters_and_bounded() {
        assert_eq!(Trigger::UNKNOWN.text(), None);
        assert_eq!(Trigger::reported("").text(), Some(""));
        assert_eq!(Trigger::reported(" Ctrl+F6\n").text(), Some("Ctrl+F6"));
        assert_eq!(
            Trigger::reported("Ctrl+\u{1b}[31mF6").text(),
            Some("Ctrl+[31mF6")
        );
        let long = Trigger::reported(&"é".repeat(500));
        assert_eq!(long.text().map(|text| text.chars().count()), Some(MAX_TEXT));
        // The key is read from the text that is kept, not from what was cut.
        let cut = format!("{}+F6", "x".repeat(MAX_TEXT));
        assert_eq!(Trigger::reported(&cut).keys, None);
    }

    #[test]
    fn a_key_no_trigger_uses_is_allowed() {
        let triggers = triggers("Ctrl+Shift+F6", "Ctrl+Shift+F7", "");
        for name in ["space", "w", "f8", "f9"] {
            assert_eq!(triggers.refusal(key(name)), None, "{name}");
        }
    }

    #[test]
    fn the_key_of_any_trigger_is_refused_whatever_its_modifiers() {
        let triggers = triggers("Ctrl+Shift+F6", "Meta+F7", "F8, Alt+F9");
        for (name, trigger) in [
            ("f6", "Ctrl+Shift+F6"),
            ("f7", "Meta+F7"),
            ("f8", "F8, Alt+F9"),
            ("f9", "F8, Alt+F9"),
        ] {
            let refusal = triggers.refusal(key(name));
            let expected = Refusal::HotkeyKey {
                key: key(name),
                trigger: trigger.to_owned(),
            };
            assert_eq!(refusal, Some(expected), "{name}");
        }
        let message = triggers.refusal(key("f6")).unwrap().to_string();
        assert!(
            message.contains("f6") && message.contains("Ctrl+Shift+F6"),
            "{message}"
        );
    }

    #[test]
    fn an_unreadable_or_unreported_trigger_refuses_every_key() {
        let unreadable = triggers("Ctrl+Shift+F6", "Ctrl+Space", "");
        assert_eq!(
            unreadable.refusal(key("w")),
            Some(Refusal::Unreadable("Ctrl+Space".to_owned()))
        );
        let message = unreadable.refusal(key("w")).unwrap().to_string();
        assert!(
            message.contains("Ctrl+Space") && message.contains("F1 to F24"),
            "{message}"
        );
        assert_eq!(
            Triggers::unknown().refusal(key("w")),
            Some(Refusal::NotReported)
        );
        // One reported trigger does not make the others known.
        let mut partly = Triggers::unknown();
        partly.set(Action::ToggleClicking, Trigger::reported("Ctrl+F6"));
        assert_eq!(partly.refusal(key("w")), Some(Refusal::NotReported));
    }

    #[test]
    fn a_modifier_key_is_refused_even_when_no_trigger_names_it() {
        let triggers = triggers("Ctrl+Shift+F6", "", "");
        for name in ["leftshift", "rightctrl", "leftalt", "rightmeta"] {
            let refusal = triggers.refusal(key(name));
            assert_eq!(refusal, Some(Refusal::Modifier(key(name))), "{name}");
        }
        let message = triggers.refusal(key("leftshift")).unwrap().to_string();
        assert!(
            message.contains("leftshift") && message.contains("modifier"),
            "{message}"
        );
    }
}
