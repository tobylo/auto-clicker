//! Matching key events from every keyboard against the bindings.

use super::{Action, Bindings, HotkeyEvent, Modifier, Modifiers};

/// One device the backend reads, as long as it is open. The backend never
/// reuses an id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeviceId(pub u64);

/// One key going down, coming up or repeating.
///
/// Deliberately without `Debug` and `Display`: a key event can never be
/// printed, so no log or error message can carry what the user typed.
pub struct KeyEvent {
    code: u16,
    value: KeyValue,
}

enum KeyValue {
    Release,
    Press,
    Repeat,
}

impl KeyEvent {
    /// The event with this key code and evdev value (0 up, 1 down, 2
    /// autorepeat), or `None` for any other value.
    pub fn new(code: u16, value: i32) -> Option<KeyEvent> {
        let value = match value {
            0 => KeyValue::Release,
            1 => KeyValue::Press,
            2 => KeyValue::Repeat,
            _ => return None,
        };
        Some(KeyEvent { code, value })
    }
}

/// What one input changed: a fired binding, and the release that follows
/// it or the news that it can no longer be seen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reaction {
    pub fired: Option<Action>,
    pub released: bool,
    pub release_lost: bool,
}

impl Reaction {
    /// The events to send, in order.
    pub fn events(self) -> impl Iterator<Item = HotkeyEvent> {
        let fired = self.fired.map(HotkeyEvent::Fired);
        let released = self.released.then_some(HotkeyEvent::ModifiersReleased);
        let lost = self.release_lost.then_some(HotkeyEvent::ReleaseLost);
        fired.into_iter().chain(released).chain(lost)
    }
}

/// The modifier state across all devices, and the bindings it is matched
/// against.
pub struct Matcher {
    bindings: Bindings,
    /// The modifier keys each device holds down, one bit per key code in
    /// [`MODIFIER_CODES`]. Devices holding none are left out.
    held: Vec<(DeviceId, u8)>,
    /// A binding fired and not every modifier was released since.
    awaiting_release: bool,
    /// Keys were read after lost events, and nothing fires until every
    /// modifier is up: events queued before the read are older than it.
    suspended: bool,
}

/// Every modifier key code, left and right, as [`Modifier::codes`] gives
/// them.
pub(super) const MODIFIER_CODES: [u16; 8] = [29, 97, 42, 54, 56, 100, 125, 126];

fn modifier_bit(code: u16) -> Option<u8> {
    MODIFIER_CODES
        .iter()
        .position(|&known| known == code)
        .map(|index| 1 << index)
}

impl Matcher {
    pub fn new(bindings: Bindings) -> Matcher {
        Matcher {
            bindings,
            held: Vec::new(),
            awaiting_release: false,
            suspended: false,
        }
    }

    /// Takes one key event from `device`.
    pub fn key(&mut self, device: DeviceId, event: KeyEvent) -> Reaction {
        match (modifier_bit(event.code), event.value) {
            (Some(bit), KeyValue::Press) => {
                self.update(device, |keys| keys | bit);
                Reaction::default()
            }
            (Some(bit), KeyValue::Release) => {
                self.update(device, |keys| keys & !bit);
                self.release_reaction()
            }
            (None, KeyValue::Press) => self.press(event.code),
            (_, KeyValue::Repeat) | (None, KeyValue::Release) => Reaction::default(),
        }
    }

    /// Forgets the keys a device held, once it is gone: unplugged keys are
    /// up.
    pub fn device_gone(&mut self, device: DeviceId) -> Reaction {
        self.forget(device);
        self.release_reaction()
    }

    /// Forgets the keys a device held after reading it failed. Nothing says
    /// they came up: when it held a modifier while a release is awaited,
    /// the release is lost instead, so no other keyboard can complete it.
    pub fn device_failed(&mut self, device: DeviceId) -> Reaction {
        let held_any = self.held.iter().any(|(held_by, _)| *held_by == device);
        self.forget(device);
        let release_lost = self.awaiting_release && held_any;
        if release_lost {
            self.awaiting_release = false;
        }
        Reaction {
            release_lost,
            ..Reaction::default()
        }
    }

    /// Replaces what a device holds with what it reports holding now, after
    /// its events were lost. Nothing fires until every modifier is up again:
    /// the read state is newer than events still queued on this device or
    /// others, so an F6 pressed before Ctrl could otherwise match it.
    pub fn resync(&mut self, device: DeviceId, held: impl Fn(u16) -> bool) -> Reaction {
        let keys = MODIFIER_CODES
            .iter()
            .enumerate()
            .filter(|&(_, &code)| held(code))
            .fold(0, |keys, (index, _)| keys | 1 << index);
        self.suspended = true;
        self.update(device, |_| keys);
        self.release_reaction()
    }

    fn forget(&mut self, device: DeviceId) {
        self.take(device);
        self.settle();
    }

    /// Ends a suspension once no modifier is held.
    fn settle(&mut self) {
        if self.held.is_empty() {
            self.suspended = false;
        }
    }

    /// The modifiers held on any device.
    pub fn modifiers(&self) -> Modifiers {
        let keys = self.held.iter().fold(0, |all, (_, keys)| all | keys);
        MODIFIER_CODES
            .iter()
            .enumerate()
            .filter(|(index, _)| keys & (1 << index) != 0)
            .filter_map(|(_, &code)| Modifier::of_code(code))
            .collect()
    }

    fn update(&mut self, device: DeviceId, change: impl Fn(u8) -> u8) {
        let keys = change(self.take(device));
        if keys != 0 {
            self.held.push((device, keys));
        }
        self.settle();
    }

    /// Removes a device's entry; returns the keys it held.
    fn take(&mut self, device: DeviceId) -> u8 {
        match self.held.iter().position(|(held_by, _)| *held_by == device) {
            Some(index) => self.held.swap_remove(index).1,
            None => 0,
        }
    }

    fn press(&mut self, code: u16) -> Reaction {
        let modifiers = self.modifiers();
        let fired =
            self.bindings.actions().into_iter().find(|(_, binding)| {
                binding.key().code() == code && binding.modifiers() == modifiers
            });
        let Some((action, _)) = fired.filter(|_| !self.suspended) else {
            return Reaction::default();
        };
        self.awaiting_release = true;
        Reaction {
            fired: Some(action),
            ..self.release_reaction()
        }
    }

    /// The release, when a fire awaits one and no modifier is held.
    fn release_reaction(&mut self) -> Reaction {
        let released = self.awaiting_release && self.held.is_empty();
        if released {
            self.awaiting_release = false;
        }
        Reaction {
            released,
            ..Reaction::default()
        }
    }
}
