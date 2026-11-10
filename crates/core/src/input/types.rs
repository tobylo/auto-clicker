//! The values the backend interface carries: buttons, keys and positions.

use std::ops::RangeInclusive;

/// Whether a button or key goes down or comes up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PressState {
    Down,
    Up,
}

/// A mouse button the program can press.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Button {
    Left,
    Right,
    Middle,
    /// The back thumb button (`BTN_SIDE`).
    Side,
    /// The forward thumb button (`BTN_EXTRA`).
    Extra,
}

impl Button {
    /// Every button, in the order a backend declares them.
    pub const ALL: [Button; 5] = [
        Button::Left,
        Button::Right,
        Button::Middle,
        Button::Side,
        Button::Extra,
    ];

    /// The Linux input event code (`BTN_*`), which libei uses too.
    pub const fn code(self) -> u16 {
        match self {
            Button::Left => 0x110,
            Button::Right => 0x111,
            Button::Middle => 0x112,
            Button::Side => 0x113,
            Button::Extra => 0x114,
        }
    }

    /// The button with this Linux input event code, if the program has it.
    pub fn from_code(code: u16) -> Option<Button> {
        Button::ALL.into_iter().find(|button| button.code() == code)
    }
}

/// A keyboard key, by its Linux input event code (`KEY_*`).
///
/// Only codes in [`Key::CODES`] exist: the standard keyboard keys from
/// `KEY_ESC` (1) to `KEY_MICMUTE` (248). The range stays clear of the `BTN_*`
/// codes (0x100 and up) so a keyboard declaring all of it is never taken for
/// a joystick or a mouse. A backend declares exactly this range, so every
/// `Key` it is given can be emitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(u16);

impl Key {
    /// Every key code a `Key` may hold.
    pub const CODES: RangeInclusive<u16> = 1..=248;

    /// The key with this code, or `None` when the code is outside
    /// [`Key::CODES`].
    pub const fn new(code: u16) -> Option<Key> {
        if code >= *Key::CODES.start() && code <= *Key::CODES.end() {
            Some(Key(code))
        } else {
            None
        }
    }

    /// The Linux input event code.
    pub const fn code(self) -> u16 {
        self.0
    }
}

/// A point on the whole output layout, in a fixed normalised range.
///
/// Each axis runs from 0 (left or top edge) to `u16::MAX` (right or bottom
/// edge); the compositor stretches that range over every output together.
/// Mapping screen pixels into it is the caller's job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AbsolutePosition {
    pub x: u16,
    pub y: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_accepts_exactly_its_code_range() {
        assert_eq!(Key::new(0), None);
        assert_eq!(Key::new(1).map(Key::code), Some(1));
        assert_eq!(Key::new(248).map(Key::code), Some(248));
        assert_eq!(Key::new(249), None);
        assert_eq!(Key::new(Button::Left.code()), None);
    }

    #[test]
    fn button_codes_match_linux_input_event_codes() {
        let codes: Vec<u16> = Button::ALL.iter().map(|b| b.code()).collect();
        assert_eq!(codes, [0x110, 0x111, 0x112, 0x113, 0x114]);
        for button in Button::ALL {
            assert_eq!(Button::from_code(button.code()), Some(button));
        }
        assert_eq!(Button::from_code(0x115), None);
    }
}
