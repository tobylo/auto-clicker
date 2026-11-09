//! Names for keyboard keys, as config files and hotkey bindings spell them.
//!
//! A name is the kernel's `KEY_*` name in lowercase without the prefix:
//! `KEY_SPACE` is "space", `KEY_F6` is "f6". Names describe key positions on a
//! US keyboard, not the label printed on the key: on a German layout, "z" is
//! the key labelled Y. Only lowercase is accepted, so every key has one
//! spelling.

use super::Key;

/// Every named key and its Linux key code, in code order.
const NAMES: &[(&str, u16)] = &[
    ("esc", 1),
    ("1", 2),
    ("2", 3),
    ("3", 4),
    ("4", 5),
    ("5", 6),
    ("6", 7),
    ("7", 8),
    ("8", 9),
    ("9", 10),
    ("0", 11),
    ("minus", 12),
    ("equal", 13),
    ("backspace", 14),
    ("tab", 15),
    ("q", 16),
    ("w", 17),
    ("e", 18),
    ("r", 19),
    ("t", 20),
    ("y", 21),
    ("u", 22),
    ("i", 23),
    ("o", 24),
    ("p", 25),
    ("leftbrace", 26),
    ("rightbrace", 27),
    ("enter", 28),
    ("leftctrl", 29),
    ("a", 30),
    ("s", 31),
    ("d", 32),
    ("f", 33),
    ("g", 34),
    ("h", 35),
    ("j", 36),
    ("k", 37),
    ("l", 38),
    ("semicolon", 39),
    ("apostrophe", 40),
    ("grave", 41),
    ("leftshift", 42),
    ("backslash", 43),
    ("z", 44),
    ("x", 45),
    ("c", 46),
    ("v", 47),
    ("b", 48),
    ("n", 49),
    ("m", 50),
    ("comma", 51),
    ("dot", 52),
    ("slash", 53),
    ("rightshift", 54),
    ("kpasterisk", 55),
    ("leftalt", 56),
    ("space", 57),
    ("capslock", 58),
    ("f1", 59),
    ("f2", 60),
    ("f3", 61),
    ("f4", 62),
    ("f5", 63),
    ("f6", 64),
    ("f7", 65),
    ("f8", 66),
    ("f9", 67),
    ("f10", 68),
    ("numlock", 69),
    ("scrolllock", 70),
    ("kp7", 71),
    ("kp8", 72),
    ("kp9", 73),
    ("kpminus", 74),
    ("kp4", 75),
    ("kp5", 76),
    ("kp6", 77),
    ("kpplus", 78),
    ("kp1", 79),
    ("kp2", 80),
    ("kp3", 81),
    ("kp0", 82),
    ("kpdot", 83),
    ("f11", 87),
    ("f12", 88),
    ("kpenter", 96),
    ("rightctrl", 97),
    ("kpslash", 98),
    ("sysrq", 99),
    ("rightalt", 100),
    ("home", 102),
    ("up", 103),
    ("pageup", 104),
    ("left", 105),
    ("right", 106),
    ("end", 107),
    ("down", 108),
    ("pagedown", 109),
    ("insert", 110),
    ("delete", 111),
    ("pause", 119),
    ("leftmeta", 125),
    ("rightmeta", 126),
    ("compose", 127),
    ("f13", 183),
    ("f14", 184),
    ("f15", 185),
    ("f16", 186),
    ("f17", 187),
    ("f18", 188),
    ("f19", 189),
    ("f20", 190),
    ("f21", 191),
    ("f22", 192),
    ("f23", 193),
    ("f24", 194),
];

impl Key {
    /// The key with this name, or `None` for a name not in the table
    /// (including any spelling that is not all lowercase).
    pub fn from_name(name: &str) -> Option<Key> {
        NAMES
            .iter()
            .find(|(known, _)| *known == name)
            .and_then(|&(_, code)| Key::new(code))
    }

    /// The key's name, or `None` for a key the table does not name.
    pub fn name(self) -> Option<&'static str> {
        NAMES
            .iter()
            .find(|&&(_, code)| code == self.code())
            .map(|&(name, _)| name)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_name_maps_to_a_key_and_back() {
        for &(name, code) in NAMES {
            let key = Key::from_name(name).unwrap();
            assert_eq!(key.code(), code, "{name}");
            assert_eq!(key.name(), Some(name));
        }
    }

    #[test]
    fn names_and_codes_are_unique_and_codes_ascend() {
        let names: HashSet<_> = NAMES.iter().map(|(name, _)| name).collect();
        assert_eq!(names.len(), NAMES.len());
        assert!(NAMES.windows(2).all(|pair| pair[0].1 < pair[1].1));
    }

    #[test]
    fn known_keys_have_their_kernel_codes() {
        let code = |name| Key::from_name(name).map(Key::code);
        assert_eq!(code("space"), Some(57));
        assert_eq!(code("w"), Some(17));
        assert_eq!(code("f6"), Some(64));
        assert_eq!(code("f24"), Some(194));
        assert_eq!(code("leftctrl"), Some(29));
    }

    #[test]
    fn other_spellings_are_not_names() {
        for name in [
            "Space",
            "SPACE",
            "KEY_SPACE",
            "key_space",
            " space",
            "",
            "ctrl",
        ] {
            assert_eq!(Key::from_name(name), None, "{name:?}");
        }
    }

    #[test]
    fn an_unnamed_key_has_no_name() {
        assert_eq!(Key::new(84).and_then(Key::name), None);
    }
}
