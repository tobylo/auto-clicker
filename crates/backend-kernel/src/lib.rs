//! The kernel input backend: a virtual mouse and keyboard through `/dev/uinput`.
//!
//! This crate is the only one that touches `/dev/uinput` and evdev. It hands
//! out an [`Input`] over its backend and never the bare backend, so every
//! press goes through the pressed set that releases it.

mod clock;
mod devices;
mod error;
mod hotkeys;

use auto_clicker_core::input::{
    AbsolutePosition, Button, Input, InputBackend, InputError, Key, PressState,
};
use evdev::uinput::VirtualDevice;
use evdev::{AbsoluteAxisCode, EventType, InputEvent, RelativeAxisCode};

pub use clock::{local_time, monotonic_now};
pub use hotkeys::{InputSources, read_input};

pub use devices::{
    ABSOLUTE_MAX, DeviceIdentity, DeviceRole, KEYBOARD_NAME, MOUSE_NAME, PRODUCT_KEYBOARD,
    PRODUCT_MOUSE, VENDOR,
};

/// The virtual mouse and keyboard.
///
/// Dropping it closes both devices; the kernel then destroys them and releases
/// any button or key still down on them, which also covers a killed process.
#[derive(Debug)]
pub struct KernelBackend {
    mouse: VirtualDevice,
    keyboard: VirtualDevice,
    identities: [DeviceIdentity; 2],
}

impl KernelBackend {
    /// Creates the virtual mouse and keyboard.
    ///
    /// # Errors
    /// [`InputError::NoPermission`] or [`InputError::UinputMissing`] when
    /// `/dev/uinput` cannot be opened, [`InputError::CreateDevice`] when a
    /// device cannot be set up. A device created before the failure is
    /// destroyed again.
    pub fn open() -> Result<Input<KernelBackend>, InputError> {
        let (mouse, mouse_identity) = devices::create(DeviceRole::Mouse)?;
        let (keyboard, keyboard_identity) = devices::create(DeviceRole::Keyboard)?;
        Ok(Input::new(KernelBackend {
            mouse,
            keyboard,
            identities: [mouse_identity, keyboard_identity],
        }))
    }

    /// Who the virtual devices are, so that hotkeys and the recorder can
    /// recognise this program's own events.
    pub fn devices(&self) -> &[DeviceIdentity] {
        &self.identities
    }
}

/// Writes one frame; `emit` ends it with `SYN_REPORT`. A zero move or scroll
/// has no events and writes a bare `SYN_REPORT`, which changes nothing.
fn emit(device: &mut VirtualDevice, events: &[InputEvent]) -> Result<(), InputError> {
    device.emit(events).map_err(InputError::Write)
}

fn key_event(code: u16, state: PressState) -> InputEvent {
    let value = match state {
        PressState::Down => 1,
        PressState::Up => 0,
    };
    InputEvent::new(EventType::KEY.0, code, value)
}

fn rel_event(axis: RelativeAxisCode, value: i32) -> InputEvent {
    InputEvent::new(EventType::RELATIVE.0, axis.0, value)
}

fn abs_event(axis: AbsoluteAxisCode, value: u16) -> InputEvent {
    InputEvent::new(EventType::ABSOLUTE.0, axis.0, i32::from(value))
}

/// The pointer motion events for one frame, leaving out a zero axis.
fn relative_events(dx: i32, dy: i32) -> Vec<InputEvent> {
    relative_axis_events(&[(RelativeAxisCode::REL_X, dx), (RelativeAxisCode::REL_Y, dy)])
}

fn relative_axis_events(axes: &[(RelativeAxisCode, i32)]) -> Vec<InputEvent> {
    axes.iter()
        .copied()
        .filter(|&(_, value)| value != 0)
        .map(|(axis, value)| rel_event(axis, value))
        .collect()
}

/// The wheel event for one frame, or none for zero notches.
fn wheel_events(notches: i32) -> Vec<InputEvent> {
    relative_axis_events(&[(RelativeAxisCode::REL_WHEEL, notches)])
}

/// The events of one absolute move.
///
/// The kernel drops an absolute value equal to the axis's last one
/// (`input_handle_abs_event` in drivers/input/input.c), so moving back to
/// the same point after the real mouse moved the pointer away would do
/// nothing. Each axis first gets a neighbouring value (lowest bit flipped,
/// always in range), then the target, in the same frame: the kernel passes
/// both, and the compositor acts only on the last value of the frame.
fn absolute_events(position: AbsolutePosition) -> [InputEvent; 4] {
    let (x, y) = (AbsoluteAxisCode::ABS_X, AbsoluteAxisCode::ABS_Y);
    [
        abs_event(x, position.x ^ 1),
        abs_event(y, position.y ^ 1),
        abs_event(x, position.x),
        abs_event(y, position.y),
    ]
}

impl InputBackend for KernelBackend {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        emit(&mut self.mouse, &[key_event(button.code(), state)])
    }

    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError> {
        emit(&mut self.keyboard, &[key_event(key.code(), state)])
    }

    fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        emit(&mut self.mouse, &relative_events(dx, dy))
    }

    fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError> {
        emit(&mut self.mouse, &absolute_events(position))
    }

    fn scroll(&mut self, notches: i32) -> Result<(), InputError> {
        emit(&mut self.mouse, &wheel_events(notches))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(events: &[InputEvent]) -> Vec<(u16, u16, i32)> {
        events
            .iter()
            .map(|e| (e.event_type().0, e.code(), e.value()))
            .collect()
    }

    #[test]
    fn key_events_carry_press_state_as_value() {
        let key = EventType::KEY.0;
        assert_eq!(summary(&[key_event(30, PressState::Down)]), [(key, 30, 1)]);
        assert_eq!(summary(&[key_event(30, PressState::Up)]), [(key, 30, 0)]);
    }

    #[test]
    fn relative_motion_leaves_out_zero_axes() {
        let rel = EventType::RELATIVE.0;
        let (x, y) = (RelativeAxisCode::REL_X.0, RelativeAxisCode::REL_Y.0);
        assert_eq!(
            summary(&relative_events(3, -4)),
            [(rel, x, 3), (rel, y, -4)]
        );
        assert_eq!(summary(&relative_events(0, 7)), [(rel, y, 7)]);
        assert_eq!(summary(&relative_events(-1, 0)), [(rel, x, -1)]);
        assert!(relative_events(0, 0).is_empty());
    }

    #[test]
    fn wheel_events_leave_out_zero_notches() {
        let rel = EventType::RELATIVE.0;
        let wheel = RelativeAxisCode::REL_WHEEL.0;
        assert_eq!(summary(&wheel_events(-2)), [(rel, wheel, -2)]);
        assert!(wheel_events(0).is_empty());
    }

    #[test]
    fn recording_names_are_evdevs_names_for_the_same_codes() {
        use std::str::FromStr;

        use auto_clicker_core::input::Key;
        use auto_clicker_core::recording::{code_name, parse_code, recordable};
        use evdev::KeyCode;

        let codes = Key::CODES.chain(Button::ALL.map(Button::code));
        let mut named = 0;
        for code in codes {
            let Some(held) = recordable(code) else {
                continue;
            };
            let name = code_name(held).unwrap();
            // Newer kernel names than evdev 0.13 knows; it has their aliases.
            let evdev_name = match name {
                "KEY_ALL_APPLICATIONS" => "KEY_DASHBOARD",
                name => name,
            };
            let evdev_code = KeyCode::from_str(evdev_name).ok();
            assert_eq!(evdev_code, Some(KeyCode(code)), "{name}");
            assert_eq!(parse_code(name), Some(held), "{name}");
            named += 1;
        }
        assert_eq!(named, 247);
    }

    #[test]
    fn absolute_moves_pass_a_neighbour_then_end_on_the_target() {
        let abs = EventType::ABSOLUTE.0;
        let (x, y) = (AbsoluteAxisCode::ABS_X.0, AbsoluteAxisCode::ABS_Y.0);
        let edge = AbsolutePosition { x: u16::MAX, y: 1 };
        assert_eq!(
            summary(&absolute_events(edge)),
            [(abs, x, 65534), (abs, y, 0), (abs, x, 65535), (abs, y, 1)]
        );
    }
}
