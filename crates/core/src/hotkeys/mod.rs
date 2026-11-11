//! Global hotkeys: the bindings, and matching key events against them.
//!
//! A backend reads the keyboards and feeds every key event to a [`Matcher`],
//! which keeps one modifier state across all of them and says when a binding
//! fires. The backend hands what the matcher says to the controller as
//! [`HotkeyEvent`]s; nothing here changes the program's mode.
//!
//! Key events are only matched, never stored or shown: [`KeyEvent`] has no
//! `Debug` or `Display`, and no [`HotkeyEvent`] carries a key but what a
//! capture the user started got, which becomes a binding once saved.

mod binding;
mod capture;
mod matcher;
mod trigger;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use binding::{Binding, Modifier, Modifiers};
pub use capture::{Capture, Captured};
pub use matcher::{DeviceId, KeyEvent, Matcher, Reaction};
pub use trigger::{DESKTOP_RELEASE_PAUSE, Refusal, Trigger, Triggers};

/// What a hotkey asks the controller to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    ToggleClicking,
    ToggleRecording,
    TogglePlayback,
}

impl Action {
    pub const ALL: [Action; 3] = [
        Action::ToggleClicking,
        Action::ToggleRecording,
        Action::TogglePlayback,
    ];
}

/// The binding of every action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bindings {
    pub toggle_clicking: Binding,
    pub toggle_recording: Binding,
    pub toggle_playback: Binding,
}

impl Default for Bindings {
    /// Ctrl+Shift+F6, Ctrl+Shift+F7 and Ctrl+Shift+F8: free in stock Plasma,
    /// whose own shortcuts take Ctrl+F7, and keys Minecraft does not use.
    fn default() -> Self {
        Bindings {
            toggle_clicking: Binding::ctrl_shift(64),
            toggle_recording: Binding::ctrl_shift(65),
            toggle_playback: Binding::ctrl_shift(66),
        }
    }
}

impl Bindings {
    /// The binding of `action`.
    pub fn binding(&self, action: Action) -> Binding {
        match action {
            Action::ToggleClicking => self.toggle_clicking,
            Action::ToggleRecording => self.toggle_recording,
            Action::TogglePlayback => self.toggle_playback,
        }
    }

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
    /// No modifier is held any more, after a [`HotkeyEvent::Fired`]: the
    /// last one was seen coming up. Comes once after every fire, right after
    /// it for a binding without modifiers; fires while modifiers stay held
    /// share one. A run started by a hotkey can wait for it, so its first
    /// clicks are not Ctrl+click. Instead of it, [`HotkeyEvent::ReleaseLost`]
    /// or a [`HotkeyStatus::Unavailable`] may end such a wait.
    ModifiersReleased,
    /// Whether the modifiers came up after a fire can no longer be known: a
    /// keyboard holding one was unplugged or failed, or a keyboard's events
    /// were lost. No release follows. A run waiting for the release should
    /// not start.
    ReleaseLost,
    /// Whether hotkeys work now. Sent once the devices were first looked at,
    /// and again whenever it changes.
    Status(HotkeyStatus),
    /// The triggers the desktop reports, from a reader that gets its hotkeys
    /// from the desktop: after it bound them, and whenever one changes.
    Triggers(Triggers),
    /// What a reader started to capture got: once, then nothing more.
    Captured(Captured),
    /// The desktop's shortcut settings were opened (`Ok`), or why not, for
    /// the command with this operation number.
    Configured {
        operation: u64,
        result: Result<(), String>,
    },
}

/// Whether hotkeys can be read: from a keyboard, or from the desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyStatus {
    Available,
    Unavailable(Unavailable),
}

/// Why hotkeys cannot be read.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
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
    /// The desktop does not deliver the hotkeys: its shortcuts service is
    /// missing, refused them or went away. Nothing else is affected.
    Desktop(String),
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
                write!(
                    f,
                    "hotkeys stopped: watching for keyboards failed: {error}; restart the program"
                )
            }
            Unavailable::Desktop(why) => write!(f, "hotkeys are unavailable: {why}"),
        }
    }
}

#[cfg(test)]
mod tests;
