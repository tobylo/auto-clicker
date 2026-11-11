//! Hotkey bindings as the config file spells them: "Ctrl+F6".
//!
//! A binding is any number of modifiers and one key, joined by `+`. Letter
//! case does not matter ("ctrl+f6" is "Ctrl+F6"); the key is a name from the
//! key-name table, such as "F6", "Space" or "W". A binding is parsed once into
//! a [`Binding`] and always shown in one form, "Ctrl+Shift+F6".

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::input::Key;

/// A modifier a binding can hold. Left and right keys count as the same
/// modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Meta,
}

impl Modifier {
    /// Every modifier, in the order a binding shows them.
    pub const ALL: [Modifier; 4] = [
        Modifier::Ctrl,
        Modifier::Shift,
        Modifier::Alt,
        Modifier::Meta,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Modifier::Ctrl => "Ctrl",
            Modifier::Shift => "Shift",
            Modifier::Alt => "Alt",
            Modifier::Meta => "Meta",
        }
    }

    /// The left and right key codes (`KEY_LEFTCTRL`, `KEY_RIGHTCTRL`, ...).
    pub const fn codes(self) -> [u16; 2] {
        match self {
            Modifier::Ctrl => [29, 97],
            Modifier::Shift => [42, 54],
            Modifier::Alt => [56, 100],
            Modifier::Meta => [125, 126],
        }
    }

    /// The modifier a key code belongs to, if any.
    pub fn of_code(code: u16) -> Option<Modifier> {
        Modifier::ALL
            .into_iter()
            .find(|modifier| modifier.codes().contains(&code))
    }

    const fn bit(self) -> u8 {
        match self {
            Modifier::Ctrl => 1,
            Modifier::Shift => 2,
            Modifier::Alt => 4,
            Modifier::Meta => 8,
        }
    }

    fn from_word(word: &str) -> Option<Modifier> {
        Modifier::ALL
            .into_iter()
            .find(|modifier| modifier.name().eq_ignore_ascii_case(word))
    }
}

/// A set of modifiers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);

    pub fn with(self, modifier: Modifier) -> Modifiers {
        Modifiers(self.0 | modifier.bit())
    }

    pub fn contains(self, modifier: Modifier) -> bool {
        self.0 & modifier.bit() != 0
    }

    /// The modifiers in the set, in display order.
    pub fn iter(self) -> impl Iterator<Item = Modifier> {
        Modifier::ALL
            .into_iter()
            .filter(move |modifier| self.contains(*modifier))
    }
}

impl FromIterator<Modifier> for Modifiers {
    fn from_iter<I: IntoIterator<Item = Modifier>>(iter: I) -> Modifiers {
        iter.into_iter().fold(Modifiers::NONE, Modifiers::with)
    }
}

/// A validated hotkey: exactly these modifiers held, then this key pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    modifiers: Modifiers,
    key: Key,
}

impl Binding {
    /// Ctrl, Shift and the key with this code, for the defaults. Fails to
    /// compile when the code is no key.
    pub(crate) const fn ctrl_shift(code: u16) -> Binding {
        match Key::new(code) {
            Some(key) => Binding {
                modifiers: Modifiers(Modifier::Ctrl.bit() | Modifier::Shift.bit()),
                key,
            },
            None => panic!("not a key code"),
        }
    }

    /// The binding of `key` with `modifiers`, on the same terms as
    /// [`Binding::parse`]: `None` for a key without a name, or a modifier
    /// key.
    pub fn new(modifiers: Modifiers, key: Key) -> Option<Binding> {
        (key.name().is_some() && Modifier::of_code(key.code()).is_none())
            .then_some(Binding { modifiers, key })
    }

    /// Parses a binding such as "Ctrl+F6". The error says what is wrong, for
    /// the config error message.
    pub fn parse(text: &str) -> Result<Binding, String> {
        if text.chars().any(char::is_whitespace) {
            return Err(format!(
                "hotkey {text:?} contains a space; write it as in \"Ctrl+F6\""
            ));
        }
        let mut parts: Vec<&str> = text.split('+').collect();
        let key_word = parts.pop().unwrap_or_default();
        let modifiers = parse_modifiers(text, &parts)?;
        let key = parse_key(text, key_word)?;
        Ok(Binding { modifiers, key })
    }

    pub fn modifiers(self) -> Modifiers {
        self.modifiers
    }

    pub fn key(self) -> Key {
        self.key
    }
}

fn parse_modifiers(text: &str, words: &[&str]) -> Result<Modifiers, String> {
    let mut modifiers = Modifiers::NONE;
    for word in words {
        let modifier = match Modifier::from_word(word) {
            Some(modifier) => modifier,
            None if word.is_empty() => return Err(empty_part(text)),
            None => {
                return Err(format!(
                    "unknown modifier {word:?} in hotkey {text:?}; modifiers are Ctrl, Shift, Alt and Meta"
                ));
            }
        };
        if modifiers.contains(modifier) {
            return Err(format!("hotkey {text:?} names {} twice", modifier.name()));
        }
        modifiers = modifiers.with(modifier);
    }
    Ok(modifiers)
}

fn parse_key(text: &str, word: &str) -> Result<Key, String> {
    if word.is_empty() {
        return Err(empty_part(text));
    }
    if Modifier::from_word(word).is_some() {
        return Err(format!(
            "hotkey {text:?} has no key after its modifiers, as in \"Ctrl+F6\""
        ));
    }
    let key = Key::from_name(&word.to_ascii_lowercase()).ok_or_else(|| {
        format!("unknown key {word:?} in hotkey {text:?}; keys are named as in \"F6\", \"Space\" or \"W\"")
    })?;
    match Modifier::of_code(key.code()) {
        None => Ok(key),
        Some(_) => Err(format!(
            "hotkey {text:?} ends in a modifier key; write the modifier as Ctrl, Shift, Alt or Meta, followed by a key"
        )),
    }
}

fn empty_part(text: &str) -> String {
    format!("hotkey {text:?} has an empty part; write it as in \"Ctrl+F6\"")
}

impl fmt::Display for Binding {
    /// "Ctrl+Shift+F6": modifiers in a fixed order, the key name capitalised.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for modifier in self.modifiers.iter() {
            write!(f, "{}+", modifier.name())?;
        }
        // Every binding's key came from a name (`parse`) or is F6 to F8
        // (`ctrl_shift`), so it has one.
        let name = self.key.name().unwrap_or_default();
        let mut chars = name.chars();
        if let Some(first) = chars.next() {
            write!(f, "{}{}", first.to_ascii_uppercase(), chars.as_str())?;
        }
        Ok(())
    }
}

/// A binding travels as its text, "Ctrl+Shift+F6", and is read back through
/// [`Binding::parse`].
impl Serialize for Binding {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Binding::parse(&text).map_err(serde::de::Error::custom)
    }
}
