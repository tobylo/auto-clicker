//! Global hotkeys: the bindings, and matching key events against them.
//!
//! A backend reads the keyboards and feeds every key event to a [`Matcher`],
//! which keeps one modifier state across all of them and says when a binding
//! fires. The backend hands what the matcher says to the controller as
//! [`HotkeyEvent`]s; nothing here changes the program's mode.
//!
//! Key events are only matched, never stored or shown: [`KeyEvent`] has no
//! `Debug` or `Display`, and no [`HotkeyEvent`] carries a key.

mod binding;
mod matcher;

use std::fmt;

pub use binding::{Binding, Modifier, Modifiers};
pub use matcher::{DeviceId, KeyEvent, Matcher, Reaction};

/// What a hotkey asks the controller to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleClicking,
    ToggleRecording,
    TogglePlayback,
}

/// The binding of every action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bindings {
    pub toggle_clicking: Binding,
    pub toggle_recording: Binding,
    pub toggle_playback: Binding,
}

impl Default for Bindings {
    /// Ctrl+F6, Ctrl+F7 and Ctrl+F8.
    fn default() -> Self {
        Bindings {
            toggle_clicking: Binding::ctrl(64),
            toggle_recording: Binding::ctrl(65),
            toggle_playback: Binding::ctrl(66),
        }
    }
}

impl Bindings {
    /// Every action and its binding.
    pub fn actions(&self) -> [(Action, Binding); 3] {
        [
            (Action::ToggleClicking, self.toggle_clicking),
            (Action::ToggleRecording, self.toggle_recording),
            (Action::TogglePlayback, self.toggle_playback),
        ]
    }

    /// Whether a device declaring these key codes is worth reading: it has a
    /// bound key or a modifier key. A keyboard split over several devices
    /// may carry its modifiers on one and the bound keys on another.
    pub fn wants_device(&self, has_key: impl Fn(u16) -> bool) -> bool {
        let bound = self.actions().map(|(_, binding)| binding.key().code());
        let modifiers = Modifier::ALL.into_iter().flat_map(Modifier::codes);
        bound.into_iter().chain(modifiers).any(has_key)
    }
}

/// What the hotkey reader tells the controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// A binding was pressed.
    Fired(Action),
    /// No modifier is held any more, after a [`HotkeyEvent::Fired`]: every
    /// modifier was seen coming up, or its keyboard was unplugged. Comes once
    /// after every fire, right after it for a binding without modifiers;
    /// fires while modifiers stay held share one. A run started by a hotkey
    /// can wait for it, so its first clicks are not Ctrl+click. It does not
    /// come when reading fails: a [`HotkeyStatus::Unavailable`] ends such a
    /// wait instead.
    ModifiersReleased,
    /// Whether hotkeys work now. Sent once the devices were first looked at,
    /// and again whenever it changes.
    Status(HotkeyStatus),
}

/// Whether any keyboard can be read for hotkeys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyStatus {
    Available,
    Unavailable(Unavailable),
}

/// Why no keyboard can be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// No input device has a bound key or a modifier key.
    NoKeyboard,
    /// Some input devices cannot be opened: the user is not in the `input`
    /// group.
    NoPermission,
    /// Keyboards were found but could not be read; the error of the last
    /// one tried. Retried when a keyboard's node changes.
    ReadFailed(String),
    /// Watching for devices failed; hotkeys stop until the daemon restarts.
    WatchFailed(String),
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unavailable::NoKeyboard => f.write_str("hotkeys are unavailable: no keyboard found"),
            Unavailable::NoPermission => f.write_str(
                "hotkeys are unavailable: keyboards cannot be read; add your user to the input group and log in again",
            ),
            Unavailable::ReadFailed(error) => {
                write!(f, "hotkeys are unavailable: a keyboard cannot be read: {error}")
            }
            Unavailable::WatchFailed(error) => {
                write!(f, "hotkeys stopped: watching for keyboards failed: {error}")
            }
        }
    }
}

#[cfg(test)]
mod tests;
