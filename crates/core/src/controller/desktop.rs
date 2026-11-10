//! Hotkeys the desktop delivers: the triggers it reports, and the runs the
//! controller refuses because of them.
//!
//! The desktop says which hotkey fired, never from which device, so the
//! controller cannot drop the program's own key output as the device reader
//! does. It keeps the program from pressing a hotkey's key at all: a run
//! that would is refused before it starts, and stopped when a trigger
//! changes under it.

use super::view::Hotkey;
use super::{Controller, InputLifetime, InputReader, Platform};
use crate::hotkeys::{Action, Bindings, Triggers};

impl<P: Platform> Controller<P> {
    /// The triggers a new controller starts with: unknown ones when the
    /// desktop delivers the hotkeys, so nothing presses a key before the
    /// desktop has said which keys are taken.
    pub(super) fn first_triggers(platform: &P) -> Option<Triggers> {
        (platform.input_reader() == InputReader::Desktop).then(Triggers::unknown)
    }

    /// Whether the reader stops while devices open and starts again after:
    /// kept devices are device nodes, which a reader of devices would read
    /// as keyboards unless it knows them.
    pub(super) fn reader_restarts(&self) -> bool {
        P::LIFETIME == InputLifetime::Kept && self.platform.input_reader() == InputReader::Devices
    }

    /// The desktop reported its triggers. A run or a start that presses a
    /// key one of them now uses ends at once.
    pub(super) fn triggers_changed(&mut self, triggers: Triggers) {
        self.triggers = Some(triggers);
        if !self.activity().clicking() {
            return;
        }
        if let Some(why) = self.own_key_refusal() {
            self.stop(Some(format!("Stopped: {why}")));
        }
    }

    /// Whether the run the config asks for may press its key; says why not
    /// when it may not.
    pub(super) fn keys_allow(&mut self) -> bool {
        match self.own_key_refusal() {
            Some(why) => {
                self.notice = Some(format!("Not started: {why}"));
                false
            }
            None => true,
        }
    }

    /// Why the configured run would set off a hotkey, if it would. Only a
    /// run that presses a key can, and only while the desktop delivers the
    /// hotkeys.
    fn own_key_refusal(&self) -> Option<String> {
        let key = self.config.clicker.pressed_key()?;
        let refusal = self.triggers.as_ref()?.refusal(key)?;
        Some(refusal.to_string())
    }

    /// Why a recording cannot start on this platform, if it cannot.
    pub(super) fn recording_refusal(&self) -> Option<&'static str> {
        (self.platform.input_reader() == InputReader::Desktop).then_some(
            "recording reads the keyboards and mice, which hotkeys = \"portal\" does not; set \
             hotkeys = \"kernel\" under [input] in the config, which needs the input group",
        )
    }

    pub(super) fn shown_hotkey(&self, action: Action) -> Hotkey {
        shown_hotkey(self.config.hotkeys, self.triggers.as_ref(), action)
    }
}

/// The hotkey of `action` as the tray names it: what the desktop reports
/// when it delivers the hotkeys, else the config's binding.
pub(super) fn shown_hotkey(
    bindings: Bindings,
    triggers: Option<&Triggers>,
    action: Action,
) -> Hotkey {
    match triggers {
        Some(triggers) => Hotkey::Desktop(triggers.of(action).text().map(str::to_owned)),
        None => Hotkey::Bound(match action {
            Action::ToggleClicking => bindings.toggle_clicking,
            Action::ToggleRecording => bindings.toggle_recording,
            Action::TogglePlayback => bindings.toggle_playback,
        }),
    }
}
