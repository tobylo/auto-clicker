//! The interface every input backend implements.

use super::{AbsolutePosition, Button, InputError, Key, PressState};

/// Writes synthetic input to the system.
///
/// Each call writes one complete input frame: its events followed by the
/// frame's end (`SYN_REPORT` on the kernel backend), so there is no separate
/// sync call. The trait is object safe, so a backend can be chosen at run time
/// as `Input<Box<dyn InputBackend + Send>>`.
///
/// Callers never hold a backend directly: they get an
/// [`Input`](super::Input), which tracks every press so that it can be
/// released. A backend's constructor returns that `Input`, not the backend.
pub trait InputBackend {
    /// Presses or releases a mouse button.
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError>;

    /// Presses or releases a keyboard key.
    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError>;

    /// Moves the pointer by a relative amount, as a mouse does. Games that lock
    /// the pointer read this.
    fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError>;

    /// Moves the pointer to a point on the output layout.
    fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError>;

    /// Turns the vertical wheel by whole notches; positive scrolls up.
    fn scroll(&mut self, notches: i32) -> Result<(), InputError>;
}

impl<B: InputBackend + ?Sized> InputBackend for Box<B> {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        (**self).button(button, state)
    }

    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError> {
        (**self).key(key, state)
    }

    fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        (**self).move_relative(dx, dy)
    }

    fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError> {
        (**self).move_absolute(position)
    }

    fn scroll(&mut self, notches: i32) -> Result<(), InputError> {
        (**self).scroll(notches)
    }
}
