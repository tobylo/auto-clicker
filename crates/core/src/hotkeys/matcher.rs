//! Matching key events from every keyboard against the bindings.

use std::time::Duration;

use super::{Action, Bindings, Capture, Captured, HotkeyEvent, Modifier, Modifiers};

/// One device the backend reads, as long as it is open. The backend never
/// reuses an id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(pub u64);

/// One key going down, coming up or repeating.
///
/// Deliberately without `Debug` and `Display`: a key event can never be
/// printed, so no log or error message can carry what the user typed.
pub struct KeyEvent {
    code: u16,
    value: KeyValue,
    time: Duration,
}

enum KeyValue {
    Release,
    Press,
    Repeat,
}

impl KeyEvent {
    /// The event with this key code and evdev value (0 up, 1 down, 2
    /// autorepeat), or `None` for any other value. `time` is when it
    /// happened, on the monotonic clock, as the kernel stamped it.
    pub fn new(code: u16, value: i32, time: Duration) -> Option<KeyEvent> {
        let value = match value {
            0 => KeyValue::Release,
            1 => KeyValue::Press,
            2 => KeyValue::Repeat,
            _ => return None,
        };
        Some(KeyEvent { code, value, time })
    }
}

/// What one input changed: a fired binding, and the release that follows
/// it or the news that it can no longer be seen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reaction {
    pub fired: Option<Action>,
    pub released: bool,
    pub release_lost: bool,
    pub captured: Option<Captured>,
}

impl Reaction {
    /// The events to send, in order.
    pub fn events(self) -> impl Iterator<Item = HotkeyEvent> {
        let fired = self.fired.map(HotkeyEvent::Fired);
        let released = self.released.then_some(HotkeyEvent::ModifiersReleased);
        let lost = self.release_lost.then_some(HotkeyEvent::ReleaseLost);
        let captured = self.captured.map(HotkeyEvent::Captured);
        fired
            .into_iter()
            .chain(released)
            .chain(lost)
            .chain(captured)
    }
}

/// The modifier state across all devices, and the bindings it is matched
/// against.
///
/// Key events arrive in the order they happened. Three changes do not: a
/// device unplugged, a device that fails, and the keys a device holds, read
/// after its events were lost. Each lands while older key events may still
/// be queued, so an older press would be matched against newer state. The
/// matcher therefore keeps the time of the last such change, and a press
/// that happened before it never fires. State changes still apply, so the
/// state is right again once the queued events are through.
pub struct Matcher {
    bindings: Bindings,
    /// The modifier keys each device holds down, one bit per key code in
    /// [`MODIFIER_CODES`]. Devices holding none are left out.
    held: Vec<(DeviceId, u8)>,
    /// A binding fired and not every modifier was released since.
    awaiting_release: bool,
    /// When the last change that came out of order happened; no press from
    /// before it fires.
    cutoff: Duration,
    /// The devices that lost events and were not read again yet: what they
    /// hold is unknown, so nothing fires.
    unread: Vec<DeviceId>,
    /// What it captures instead of matching, if it does.
    capture: Option<Capture>,
    /// The capture got what it was for: it reports nothing more.
    captured: bool,
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
            cutoff: Duration::ZERO,
            unread: Vec::new(),
            capture: None,
            captured: false,
        }
    }

    /// A matcher that fires no binding and captures what the user presses
    /// for `capture`, once.
    pub fn capturing(capture: Capture) -> Matcher {
        Matcher {
            capture: Some(capture),
            ..Matcher::new(Bindings::default())
        }
    }

    /// Takes one key event from `device`.
    pub fn key(&mut self, device: DeviceId, event: KeyEvent) -> Reaction {
        if let Some(capture) = self.capture {
            return self.capture_key(capture, device, &event);
        }
        match (modifier_bit(event.code), event.value) {
            (Some(bit), KeyValue::Press) => {
                self.update(device, |keys| keys | bit);
                Reaction::default()
            }
            (Some(bit), KeyValue::Release) => {
                self.update(device, |keys| keys & !bit);
                self.release_reaction()
            }
            (None, KeyValue::Press) => self.press(event.code, event.time),
            (_, KeyValue::Repeat) | (None, KeyValue::Release) => Reaction::default(),
        }
    }

    /// A key event while capturing: modifiers are tracked as always, and
    /// the first press that ends the capture is reported. A press from
    /// before the last out-of-order change, or while a device's state is
    /// unknown, is not taken: what was held with it is not known.
    fn capture_key(&mut self, capture: Capture, device: DeviceId, event: &KeyEvent) -> Reaction {
        if let Some(bit) = modifier_bit(event.code) {
            match event.value {
                KeyValue::Press => self.update(device, |keys| keys | bit),
                KeyValue::Release => self.update(device, |keys| keys & !bit),
                KeyValue::Repeat => {}
            }
        }
        let pressed = matches!(event.value, KeyValue::Press);
        let captured = (pressed && !self.captured && self.may_fire(event.time))
            .then(|| capture.press(event.code, self.modifiers()))
            .flatten();
        self.captured = self.captured || captured.is_some();
        Reaction {
            captured,
            ..Reaction::default()
        }
    }

    /// Forgets the keys a device held once it is unplugged or cannot be
    /// read, `now`. Its keys are not seen coming up: if it held a modifier
    /// while a release is awaited, the release is lost rather than claimed.
    pub fn device_lost(&mut self, device: DeviceId, now: Duration) -> Reaction {
        self.unread.retain(|unread| *unread != device);
        self.cutoff = self.cutoff.max(now);
        let held_any = self.take(device) != 0;
        self.lose_release_if(held_any)
    }

    /// A device's events were lost. Nothing fires until it is read again
    /// with [`Matcher::resync`], and an awaited release is lost.
    pub fn events_lost(&mut self, device: DeviceId) -> Reaction {
        if !self.unread.contains(&device) {
            self.unread.push(device);
        }
        self.lose_release_if(true)
    }

    /// Replaces what a device holds with what it reports holding `now`,
    /// after its events were lost.
    pub fn resync(&mut self, device: DeviceId, held: impl Fn(u16) -> bool, now: Duration) {
        let keys = MODIFIER_CODES
            .iter()
            .enumerate()
            .filter(|&(_, &code)| held(code))
            .fold(0, |keys, (index, _)| keys | 1 << index);
        self.unread.retain(|unread| *unread != device);
        self.cutoff = self.cutoff.max(now);
        self.update(device, |_| keys);
    }

    fn lose_release_if(&mut self, uncertain: bool) -> Reaction {
        Reaction {
            release_lost: self.end_wait(uncertain),
            ..Reaction::default()
        }
    }

    /// Ends an awaited release when `now`; returns whether it ended.
    fn end_wait(&mut self, now: bool) -> bool {
        let ended = self.awaiting_release && now;
        if ended {
            self.awaiting_release = false;
        }
        ended
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
    }

    /// Removes a device's entry; returns the keys it held.
    fn take(&mut self, device: DeviceId) -> u8 {
        match self.held.iter().position(|(held_by, _)| *held_by == device) {
            Some(index) => self.held.swap_remove(index).1,
            None => 0,
        }
    }

    /// Whether a press that happened at `time` may fire: no device's state
    /// is unknown, and it is not older than the last change that came out
    /// of order.
    fn may_fire(&self, time: Duration) -> bool {
        self.unread.is_empty() && time >= self.cutoff
    }

    fn press(&mut self, code: u16, time: Duration) -> Reaction {
        let modifiers = self.modifiers();
        let fired =
            self.bindings.actions().into_iter().find(|(_, binding)| {
                binding.key().code() == code && binding.modifiers() == modifiers
            });
        let Some((action, _)) = fired.filter(|_| self.may_fire(time)) else {
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
        Reaction {
            released: self.end_wait(self.held.is_empty()),
            ..Reaction::default()
        }
    }
}
