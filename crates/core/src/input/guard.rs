//! The only way to press: [`Input`] tracks every press and releases it.

use super::pressed::PressedSet;
use super::{AbsolutePosition, Button, Held, InputBackend, InputError, Key, PressState};

/// Owns an input backend and every button and key pressed through it.
///
/// A button or key is recorded as held before its press is written, so a press
/// whose write failed is still released. [`Input::release_all`] and `Drop`
/// release exactly what is held, nothing else. Dropping during a panic
/// releases too, since the workspace builds with `panic = "unwind"`.
#[derive(Debug)]
pub struct Input<B: InputBackend> {
    backend: B,
    held: PressedSet,
}

impl<B: InputBackend> Input<B> {
    /// Wraps a backend. A backend calls this from its own constructor and
    /// returns the result; callers never hold the bare backend.
    pub fn new(backend: B) -> Self {
        Input {
            backend,
            held: PressedSet::default(),
        }
    }

    /// Read-only access to the backend, for what it reports about itself
    /// (device identities, say). Presses cannot go through a shared reference.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn press_button(&mut self, button: Button) -> Result<(), InputError> {
        self.press(Held::Button(button))
    }

    pub fn release_button(&mut self, button: Button) -> Result<(), InputError> {
        self.release(Held::Button(button))
    }

    pub fn press_key(&mut self, key: Key) -> Result<(), InputError> {
        self.press(Held::Key(key))
    }

    pub fn release_key(&mut self, key: Key) -> Result<(), InputError> {
        self.release(Held::Key(key))
    }

    pub fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        self.backend.move_relative(dx, dy)
    }

    pub fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError> {
        self.backend.move_absolute(position)
    }

    pub fn scroll(&mut self, notches: i32) -> Result<(), InputError> {
        self.backend.scroll(notches)
    }

    /// Whether `item` is held: pressed and not yet released successfully.
    pub fn is_held(&self, item: Held) -> bool {
        self.held.contains(item)
    }

    /// Everything held, in release order.
    pub fn held(&self) -> impl Iterator<Item = Held> + '_ {
        self.held.iter()
    }

    /// Releases every held button and key.
    ///
    /// Tries every release even when one fails, and returns the first error.
    /// An item whose release failed stays held, so the next call (or `Drop`)
    /// tries it again.
    pub fn release_all(&mut self) -> Result<(), InputError> {
        let mut first_error = None;
        for item in self.held.snapshot() {
            if let Err(err) = self.release(item) {
                first_error.get_or_insert(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn press(&mut self, item: Held) -> Result<(), InputError> {
        self.held.insert(item);
        self.write(item, PressState::Down)
    }

    /// Writes the release even when `item` is not held: the caller asked for
    /// it, and the kernel ignores a release of something that is up.
    fn release(&mut self, item: Held) -> Result<(), InputError> {
        self.write(item, PressState::Up)?;
        self.held.remove(item);
        Ok(())
    }

    fn write(&mut self, item: Held, state: PressState) -> Result<(), InputError> {
        match item {
            Held::Button(button) => self.backend.button(button, state),
            Held::Key(key) => self.backend.key(key, state),
        }
    }
}

impl<B: InputBackend> Drop for Input<B> {
    /// Releases everything held. A failure cannot be reported from here, and
    /// panicking during an unwind would abort before the remaining releases,
    /// so errors are dropped; [`Input::release_all`] is the path that reports
    /// them. On the kernel backend, the devices close right after this and the
    /// kernel releases whatever is still down.
    fn drop(&mut self) {
        let _ = self.release_all();
    }
}
