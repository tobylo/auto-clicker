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

use serde::{Deserialize, Serialize};

use super::Action;
use crate::input::Key;
use crate::message::Quoted;

/// How long after the desktop reports a hotkey released a run it started
/// begins. The desktop reports the release at the first key lifted, usually
/// the function key while Ctrl and Shift are still down, and never says when
/// those come up; the pause keeps the first clicks from being Ctrl+clicks.
pub const DESKTOP_RELEASE_PAUSE: Duration = Duration::from_secs(1);

/// The longest trigger text kept, in characters.
const MAX_TEXT: usize = 80;

/// The longest description whose triggers are read, in bytes: room for a
/// few dozen.
const MAX_DESCRIPTION: usize = 1024;

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
        let shown: String = description
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_TEXT)
            .collect();
        // The keys come from the whole description, never from the part
        // that fits: a text cut short could end in another key ("F12" cut
        // to "F1") or lose a trigger.
        Trigger {
            text: Some(shown.trim().to_owned()),
            keys: keys_of(description),
        }
    }

    /// The desktop's text: `None` until it reported one, empty when the
    /// action has no trigger.
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }
}

/// The key of every trigger in `description` ("Ctrl+F6, Meta+F7"), or
/// `None` when one of them is not words joined by `+` that end in a
/// function key. Anything else between the triggers (another separator, a
/// word of some language) leaves the keys unknown, never fewer.
fn keys_of(description: &str) -> Option<Vec<Key>> {
    if description.is_empty() {
        return Some(Vec::new());
    }
    // A description far longer than any list of triggers is not worked
    // through: its keys stay unknown.
    if description.len() > MAX_DESCRIPTION {
        return None;
    }
    description.split(", ").map(function_key).collect()
}

/// The function key a trigger such as "Ctrl+Shift+F6" ends in.
fn function_key(trigger: &str) -> Option<Key> {
    let mut words = trigger.split('+');
    let word = words.next_back()?;
    if !words.all(|modifier| !modifier.is_empty() && modifier.chars().all(char::is_alphanumeric)) {
        return None;
    }
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
    /// hotkey, or, held by the program, change which hotkey the user's own
    /// press sets off.
    pub fn refusal(&self, key: Key) -> Option<Refusal> {
        if super::Modifier::of_code(key.code()).is_some() {
            return Some(Refusal::Modifier(key));
        }
        self.hotkey_refusal(key)
    }

    /// Why a run may not press `key` because it could set off a hotkey, if
    /// it could. Unlike [`Triggers::refusal`], a modifier passes: a
    /// recording that holds Ctrl or Shift is what the user did.
    pub fn hotkey_refusal(&self, key: Key) -> Option<Refusal> {
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
                trigger: text.clone().unwrap_or_default().into(),
            }),
            (None, Some(text)) => Some(Refusal::Unreadable(text.clone().into())),
            (None, None) => Some(Refusal::NotReported),
        }
    }
}

/// Why a run that presses a key may not start while the desktop delivers
/// the hotkeys.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Refusal {
    /// The key is a modifier: held by the program, it would turn the user's
    /// Ctrl+F6 into Ctrl+Shift+F6.
    Modifier(Key),
    /// The key is the key of this trigger.
    HotkeyKey { key: Key, trigger: Quoted },
    /// This trigger does not end in a function key, so its key is unknown.
    Unreadable(Quoted),
    /// The desktop has not reported the triggers yet.
    NotReported,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Modifier(key) => write!(
                f,
                "{} is a modifier, and the program holding it would change which hotkey the \
                 desktop sees",
                key_name(*key)
            ),
            Refusal::HotkeyKey { key, trigger } => write!(
                f,
                "{} is the key of the hotkey {trigger}, and the desktop cannot tell the \
                 program's key presses from yours",
                key_name(*key)
            ),
            Refusal::Unreadable(trigger) => write!(
                f,
                "the hotkey \u{201c}{trigger}\u{201d} is not on a function key (F1 to F24), so \
                 the program cannot tell whether its own key presses would set it off; change \
                 it in System Settings"
            ),
            Refusal::NotReported => f.write_str(
                "the desktop has not said yet which keys the hotkeys use, and the program must \
                 not press one of them",
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
        let cases: [(&str, Option<&[&str]>); 22] = [
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
            // Another separator or stray text never hides a trigger.
            ("Ctrl+Shift+F6; Meta+F9", None),
            ("Ctrl+Shift+F6 or Meta+F9", None),
            ("Ctrl+F6,Meta+F9", None),
            ("Ctrl+F6 ", None),
            (" F6", None),
            ("Ctrl + F6", None),
            ("+F6", None),
            ("Ctrl+F6\n", None),
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
        // Eighty spaces and then a trigger: shown as nothing, but not read
        // as "no trigger".
        let hidden = format!("{}Ctrl+F6", " ".repeat(MAX_TEXT));
        assert_eq!(Trigger::reported(&hidden).text(), Some(""));
        assert_eq!(Trigger::reported(&hidden).keys, None);
        assert_eq!(
            Trigger::reported("Ctrl+\u{1b}[31mF6").text(),
            Some("Ctrl+[31mF6")
        );
        let long = Trigger::reported(&"é".repeat(500));
        assert_eq!(long.text().map(|text| text.chars().count()), Some(MAX_TEXT));
        // A cut never changes which keys are read: "F12" cut to "F1", and
        // a last trigger cut off whole.
        let long =
            "Ctrl+Shift+Alt+Meta+F24, Ctrl+Shift+Alt+Meta+F6, Ctrl+Shift+Alt+Meta+F7, Ctrl+F12";
        let cut = Trigger::reported(long);
        assert!(cut.text().unwrap().ends_with("Ctrl+F1"), "{:?}", cut.text());
        assert_eq!(
            cut.keys,
            Some(vec![key("f24"), key("f6"), key("f7"), key("f12")])
        );
        // A description too long to be a list of triggers is not read.
        let endless = vec!["F24"; 1000].join(", ");
        assert!(endless.len() > MAX_DESCRIPTION);
        assert_eq!(Trigger::reported(&endless).keys, None);
        // Exactly the longest that is read, and one byte more.
        let longest = format!("{}+F6", "A".repeat(MAX_DESCRIPTION - 3));
        assert_eq!(longest.len(), MAX_DESCRIPTION);
        assert_eq!(Trigger::reported(&longest).keys, Some(vec![key("f6")]));
        assert_eq!(Trigger::reported(&format!("A{longest}")).keys, None);
        let fits = vec!["Ctrl+Shift+F24"; 60].join(", ");
        assert!(fits.len() <= MAX_DESCRIPTION);
        assert_eq!(
            Trigger::reported(&fits).keys.map(|keys| keys.len()),
            Some(60)
        );
        let dropped =
            "Ctrl+Alt+Meta+F1, Ctrl+Shift+F10, Ctrl+Shift+F11, Ctrl+Shift+F12, Ctrl+Shift+F13, F9";
        let keys = Trigger::reported(dropped).keys.unwrap();
        assert!(keys.contains(&key("f9")), "{keys:?}");
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
                trigger: trigger.to_owned().into(),
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
            Some(Refusal::Unreadable("Ctrl+Space".to_owned().into()))
        );
        assert_eq!(
            unreadable.hotkey_refusal(key("w")),
            unreadable.refusal(key("w"))
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
            // Only as a key the program holds by itself: a recording may.
            assert_eq!(triggers.hotkey_refusal(key(name)), None, "{name}");
        }
        let message = triggers.refusal(key("leftshift")).unwrap().to_string();
        assert!(
            message.contains("leftshift") && message.contains("modifier"),
            "{message}"
        );
    }
}
