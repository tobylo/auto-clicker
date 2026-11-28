//! Hotkey capture in the window (M5 brief decision 9): while the window
//! waits on a capture, the daemon has let go of its hotkeys, and the keys
//! the user presses reach this window as its own key events. They are read
//! here, before Slint sees them, so no key the user presses for a hotkey
//! also types into a field or presses a button. A key whose press the
//! capture took stays away from Slint until it comes up, repeats included,
//! even once the capture is over, or until the window loses the keyboard:
//! a key still held when the keyboard comes back is one held as the window
//! got it, whose press Slint is handed like any other. A key Slint saw go
//! down, before the capture, it also sees come up, so it never thinks a
//! key is held that is not.
//!
//! A key is told by the scan code Windows reports (winit's physical key),
//! read back through the same table output and raw input use, so a hotkey
//! captured here is the one the daemon matches. The modifiers are those
//! Windows says are held. What is captured goes to the daemon as the
//! window's answer; like every key event, it is never logged.

use auto_clicker_backend_windows::window_message_key;
use auto_clicker_core::hotkeys::{Capture, Modifier, Modifiers};
use auto_clicker_core::window::Pressed;
use i_slint_backend_winit::winit::event::{ElementState, KeyEvent, WindowEvent};
use i_slint_backend_winit::winit::event_loop::ActiveEventLoop;
use i_slint_backend_winit::winit::keyboard::{ModifiersState, PhysicalKey};
use i_slint_backend_winit::winit::platform::scancode::PhysicalKeyExtScancode;
use i_slint_backend_winit::winit::window::{Window, WindowId};
use i_slint_backend_winit::{CustomApplicationHandler, EventResult};

use crate::APP;

/// Reads the window's key events for a capture, while one is waited on.
#[derive(Default)]
pub struct Keys {
    /// The modifiers Windows last said are held.
    modifiers: Modifiers,
    /// The keys whose press a capture took, until each comes up.
    taken: Vec<PhysicalKey>,
}

impl CustomApplicationHandler for Keys {
    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        _winit_window: Option<&Window>,
        _slint_window: Option<&slint::Window>,
        event: &WindowEvent,
    ) -> EventResult {
        match event {
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = held(modifiers.state());
                EventResult::Propagate
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                let (taken, pressed) = self.key(event, *is_synthetic, capturing());
                if let Some(pressed) = pressed {
                    tell(pressed);
                }
                if taken {
                    EventResult::PreventDefault
                } else {
                    EventResult::Propagate
                }
            }
            // The keyboard went to another window: what the user presses
            // there is no capture's, so it ends now and the daemon takes
            // its hotkeys back at once. No key is held here any more.
            WindowEvent::Focused(false) => {
                self.taken.clear();
                if capturing().is_some() {
                    tell(Pressed::Cancelled);
                }
                EventResult::Propagate
            }
            _ => EventResult::Propagate,
        }
    }
}

impl Keys {
    /// Whether `event` stays away from Slint, and what it captures for
    /// `capture`, the capture waited on, if any.
    fn key(
        &mut self,
        event: &KeyEvent,
        synthetic: bool,
        capture: Option<Capture>,
    ) -> (bool, Option<Pressed>) {
        let down = event.state == ElementState::Pressed;
        let fresh = down && !event.repeat && !synthetic;
        self.route(event.physical_key, down, fresh, capture)
    }

    /// [`Keys::key`] for `key` going down or up; `fresh` when it went
    /// down just now, not a repeat nor held as the window got the
    /// keyboard.
    fn route(
        &mut self,
        key: PhysicalKey,
        down: bool,
        fresh: bool,
        capture: Option<Capture>,
    ) -> (bool, Option<Pressed>) {
        let taken = self.taken.iter().position(|&held| held == key);
        match (down, taken, capture) {
            // Up: Slint sees it only if it saw it go down.
            (false, Some(at), _) => {
                self.taken.swap_remove(at);
                (true, None)
            }
            (true, Some(_), _) => (true, None),
            (true, None, Some(capture)) if fresh => {
                self.taken.push(key);
                (true, pressed(capture, key, self.modifiers))
            }
            _ => (false, None),
        }
    }
}

/// What a capture of `capture` gets from a press of `key` with `modifiers`
/// held; `None` for a press that does not end it. A key without a code the
/// program knows ends a hotkey's capture as one without a name.
fn pressed(capture: Capture, key: PhysicalKey, modifiers: Modifiers) -> Option<Pressed> {
    let known = key
        .to_scancode()
        .and_then(window_message_key)
        .map(|key| key.code());
    match known {
        Some(code) => capture.press(code, modifiers).map(Pressed::from),
        None => (capture == Capture::Hotkey).then_some(Pressed::Unnamed),
    }
}

/// The modifiers of `state`.
fn held(state: ModifiersState) -> Modifiers {
    [
        (state.control_key(), Modifier::Ctrl),
        (state.shift_key(), Modifier::Shift),
        (state.alt_key(), Modifier::Alt),
        (state.super_key(), Modifier::Meta),
    ]
    .into_iter()
    .filter(|&(down, _)| down)
    .fold(Modifiers::NONE, |all, (_, modifier)| all.with(modifier))
}

/// What the capture the window waits on reads, if it waits on one.
fn capturing() -> Option<Capture> {
    APP.with(|slot| slot.borrow().as_ref()?.try_borrow().ok()?.capturing())
}

fn tell(pressed: Pressed) {
    APP.with(|slot| {
        if let Some(app) = slot.borrow().as_ref()
            && let Ok(mut app) = app.try_borrow_mut()
        {
            app.pressed(pressed);
        }
    });
}

#[cfg(test)]
mod tests {
    use auto_clicker_core::hotkeys::Binding;
    use auto_clicker_core::input::Key;
    use i_slint_backend_winit::winit::keyboard::KeyCode;

    use super::*;

    fn code(key: KeyCode) -> PhysicalKey {
        PhysicalKey::Code(key)
    }

    #[test]
    fn a_hotkeys_capture_takes_the_modifiers_held_and_the_key() {
        let ctrl_shift = Modifiers::NONE.with(Modifier::Ctrl).with(Modifier::Shift);
        let f9 = Key::new(67).unwrap();
        assert_eq!(
            pressed(Capture::Hotkey, code(KeyCode::F9), ctrl_shift),
            Some(Pressed::Hotkey(Binding::new(ctrl_shift, f9).unwrap()))
        );
        // A modifier alone ends nothing; Esc cancels.
        assert_eq!(
            pressed(Capture::Hotkey, code(KeyCode::ControlRight), ctrl_shift),
            None
        );
        assert_eq!(
            pressed(Capture::Hotkey, code(KeyCode::Escape), Modifiers::NONE),
            Some(Pressed::Cancelled)
        );
    }

    #[test]
    fn the_clickers_key_may_be_a_modifier_and_keypad_keys_are_their_own() {
        let key = |code: u16| Some(Pressed::Key(Key::new(code).unwrap()));
        let none = Modifiers::NONE;
        assert_eq!(
            pressed(Capture::Key, code(KeyCode::ShiftLeft), none),
            key(42)
        );
        assert_eq!(
            pressed(Capture::Key, code(KeyCode::NumpadEnter), none),
            key(96)
        );
        assert_eq!(pressed(Capture::Key, code(KeyCode::Enter), none), key(28));
        assert_eq!(pressed(Capture::Key, code(KeyCode::Pause), none), key(119));
        assert_eq!(pressed(Capture::Key, code(KeyCode::NumLock), none), key(69));
    }

    #[test]
    fn a_key_the_program_has_no_code_for_has_no_name() {
        let unknown = PhysicalKey::Unidentified(
            i_slint_backend_winit::winit::keyboard::NativeKeyCode::Unidentified,
        );
        assert_eq!(
            pressed(Capture::Hotkey, unknown, Modifiers::NONE),
            Some(Pressed::Unnamed)
        );
        assert_eq!(pressed(Capture::Key, unknown, Modifiers::NONE), None);
    }

    #[test]
    fn a_key_the_capture_took_stays_away_from_slint_until_it_comes_up() {
        let mut keys = Keys::default();
        let space = code(KeyCode::Space);
        let ctrl = code(KeyCode::ControlLeft);
        let hotkey = Some(Capture::Hotkey);
        // Ctrl went down before the capture: Slint sees it come up.
        assert_eq!(keys.route(ctrl, true, true, None), (false, None));
        assert_eq!(keys.route(ctrl, true, false, hotkey), (false, None));
        assert_eq!(keys.route(ctrl, false, false, hotkey), (false, None));
        // Space is captured; its repeats and its release, after the
        // capture is over, stay away too, and are captured no more.
        let space_key = Some(Pressed::Key(Key::new(57).unwrap()));
        let key = Some(Capture::Key);
        assert_eq!(keys.route(space, true, true, key), (true, space_key));
        assert_eq!(keys.route(space, true, false, None), (true, None));
        assert_eq!(keys.route(space, true, false, key), (true, None));
        assert_eq!(keys.route(space, false, false, None), (true, None));
        // Up once: the next press is Slint's again.
        assert_eq!(keys.route(space, true, true, None), (false, None));
        assert_eq!(keys.route(space, false, false, None), (false, None));
    }

    #[test]
    fn windows_modifiers_are_the_programs() {
        let state = ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SUPER;
        let expected = Modifiers::NONE
            .with(Modifier::Ctrl)
            .with(Modifier::Alt)
            .with(Modifier::Meta);
        assert_eq!(held(state), expected);
        assert_eq!(held(ModifiersState::empty()), Modifiers::NONE);
    }
}
