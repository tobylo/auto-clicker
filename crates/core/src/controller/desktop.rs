//! Hotkeys the desktop delivers: the triggers it reports, and the runs the
//! controller refuses because of them.
//!
//! The desktop says which hotkey fired, never from which device, so the
//! controller cannot drop the program's own key output as the device reader
//! does. It keeps the program from pressing a hotkey's key at all: a run
//! that would is refused before it starts, and stopped when a trigger
//! changes under it.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use super::view::Hotkey;
use super::{Controller, InputLifetime, InputReader, Job, Platform, State};
use crate::config::Config;
use crate::hotkeys::{Action, Bindings, HotkeyStatus, Triggers, Unavailable};
use crate::input::Key;

/// The keys a run presses by itself, which the desktop would take for the
/// user's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum OwnKeys {
    /// The key the clicker repeats or holds, in the modes that press one.
    /// It may not be a modifier either: held by the program alone, it would
    /// change which hotkey the user's own press sets off.
    Clicker(Option<Key>),
    /// Every key a recording presses. Modifiers among them are what the
    /// user did, and stay.
    Recording(Arc<[Key]>),
}

impl Job {
    /// The keys this job presses.
    pub(super) fn own_keys(&self, config: &Config) -> OwnKeys {
        match self {
            Job::Click => OwnKeys::Clicker(config.clicker.pressed_key()),
            Job::Play(playing) => OwnKeys::Recording(playing.keys()),
        }
    }
}

/// The hotkeys of a desktop that delivers them.
pub(super) struct DesktopHotkeys {
    /// The triggers it last reported.
    triggers: Triggers,
    /// Whether it may be delivering them. Not while the reader says the
    /// desktop's shortcuts are unavailable: nothing can fire then, whatever
    /// the program presses, until the reader reports triggers again.
    live: bool,
    /// Until when a hotkey is taken for the echo of the program's own keys
    /// and does nothing: see [`ECHO_WINDOW`].
    echo_until: Option<Instant>,
}

/// How long after the desktop's shortcuts come back a hotkey does nothing.
///
/// While they were away, a run may press any key. The desktop binds the
/// shortcuts a moment before the program hears of it, and a key the run
/// pressed in that moment can set one off: the activation is already on its
/// way when the triggers arrive and stop the run. It follows them within
/// milliseconds, and nothing the program runs is left to stop by then, so a
/// hotkey this soon after is not the user's.
///
/// The desktop stamps no time on an activation (always 0 on Plasma), so the
/// window is measured from when the program hears of the return. An echo
/// the program gets to more than a second late, because the whole process
/// stood still that long in that very moment, counts as the user's.
pub(super) const ECHO_WINDOW: Duration = Duration::from_secs(1);

impl<P: Platform> Controller<P> {
    /// The triggers a new controller starts with: unknown ones when the
    /// desktop delivers the hotkeys, so nothing presses a key before the
    /// desktop has said which keys are taken.
    pub(super) fn first_triggers(platform: &P) -> Option<DesktopHotkeys> {
        (platform.input_reader() == InputReader::Desktop).then(|| DesktopHotkeys {
            triggers: Triggers::unknown(),
            live: true,
            echo_until: None,
        })
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
        // Back after being away: what the program pressed meanwhile may
        // have set off a hotkey of the new session.
        // A later report (a trigger changed in the desktop's settings)
        // leaves a window that is open as it is.
        let echo_until = match &self.desktop {
            Some(desktop) if desktop.live => desktop.echo_until,
            _ => Instant::now().checked_add(ECHO_WINDOW),
        };
        self.desktop = Some(DesktopHotkeys {
            triggers,
            live: true,
            echo_until,
        });
        let Some(keys) = self.job_keys() else {
            return;
        };
        if let Some(why) = self.own_key_refusal(&keys) {
            self.stop(Some(format!("Stopped: {why}")));
        }
    }

    /// Whether a hotkey that fired now is taken for the echo of the
    /// program's own keys.
    pub(super) fn in_echo_window(&self) -> bool {
        self.desktop
            .as_ref()
            .and_then(|desktop| desktop.echo_until)
            .is_some_and(|until| Instant::now() < until)
    }

    /// The keys of the run that is going or about to start, if one is.
    fn job_keys(&self) -> Option<OwnKeys> {
        match &self.state {
            State::Countdown(_, job) | State::Release(_, job) | State::Opening(job) => {
                Some(job.own_keys(&self.config))
            }
            State::Running(running) => Some(running.keys.clone()),
            State::Idle | State::RecordCountdown(_) | State::Recording(_) => None,
        }
    }

    /// Whether the run the config asks for may press its key; says why not
    /// when it may not.
    pub(super) fn keys_allow(&mut self) -> bool {
        match self.own_key_refusal(&Job::Click.own_keys(&self.config)) {
            Some(why) => {
                self.notice = Some(format!("Not started: {why}"));
                false
            }
            None => true,
        }
    }

    /// The desktop's shortcuts became unavailable: until the reader reports
    /// triggers again, the desktop delivers no hotkey.
    pub(super) fn desktop_status_changed(&mut self, status: &HotkeyStatus) {
        if let (Some(desktop), HotkeyStatus::Unavailable(Unavailable::Desktop(_))) =
            (&mut self.desktop, status)
        {
            desktop.live = false;
        }
    }

    /// Why a run pressing `keys` would set off a hotkey, if it would. It
    /// can only while the desktop delivers the hotkeys.
    pub(super) fn own_key_refusal(&self, keys: &OwnKeys) -> Option<String> {
        let desktop = self.desktop.as_ref().filter(|desktop| desktop.live)?;
        let refusal = match keys {
            OwnKeys::Clicker(key) => desktop.triggers.refusal((*key)?),
            OwnKeys::Recording(keys) => keys
                .iter()
                .find_map(|key| desktop.triggers.hotkey_refusal(*key)),
        }?;
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
        shown_hotkey(self.config.hotkeys, self.desktop.as_ref(), action)
    }
}

/// The hotkey of `action` as the tray names it: what the desktop reports
/// when it delivers the hotkeys, else the config's binding.
pub(super) fn shown_hotkey(
    bindings: Bindings,
    desktop: Option<&DesktopHotkeys>,
    action: Action,
) -> Hotkey {
    match desktop {
        Some(desktop) => Hotkey::Desktop(desktop.triggers.of(action).text().map(str::to_owned)),
        None => Hotkey::Bound(match action {
            Action::ToggleClicking => bindings.toggle_clicking,
            Action::ToggleRecording => bindings.toggle_recording,
            Action::TogglePlayback => bindings.toggle_playback,
        }),
    }
}
