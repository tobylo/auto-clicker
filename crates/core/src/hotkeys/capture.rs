//! Capturing what the user presses, for the settings window: the reader
//! reads every keyboard, fires no hotkey, and reports the first chord.
//!
//! What it captures is kept only as the binding or key the user then saves;
//! like every key event, it is never logged.

use serde::{Deserialize, Serialize};

use super::{Binding, Modifier, Modifiers};
use crate::input::Key;

/// What a capture is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capture {
    /// A hotkey: the modifiers held when a key other than a modifier goes
    /// down, and that key.
    Hotkey,
    /// The key the clicker presses: the first key that goes down, a
    /// modifier too.
    Key,
}

/// The `KEY_ESC` code, which ends a capture without a result.
const ESC: u16 = 1;

/// What a capture got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Captured {
    Hotkey(Binding),
    Key(Key),
    /// Esc was pressed: the user changed their mind.
    Cancelled,
    /// A key without a name, which no hotkey can use.
    Unnamed,
}

impl Capture {
    /// What a press of `code` with `modifiers` held captures; `None` for a
    /// press that does not end the capture: a modifier while capturing a
    /// hotkey, or a button that is no key.
    pub(super) fn press(self, code: u16, modifiers: Modifiers) -> Option<Captured> {
        let key = Key::new(code)?;
        if code == ESC {
            return Some(Captured::Cancelled);
        }
        match self {
            Capture::Key => Some(Captured::Key(key)),
            Capture::Hotkey if Modifier::of_code(code).is_some() => None,
            Capture::Hotkey => {
                Some(Binding::new(modifiers, key).map_or(Captured::Unnamed, Captured::Hotkey))
            }
        }
    }
}
