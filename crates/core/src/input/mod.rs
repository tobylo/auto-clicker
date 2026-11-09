//! Synthetic input: the backend interface and the guard that releases every
//! button and key the program pressed.
//!
//! Callers press through [`Input`] only. A backend implements
//! [`InputBackend`] and hands out nothing but an `Input` wrapping itself, so
//! no press can bypass the pressed set that [`Input::release_all`] and `Drop`
//! work from.

mod backend;
mod error;
#[cfg(any(test, feature = "fake"))]
pub mod fake;
mod guard;
mod pressed;
mod types;

pub use backend::InputBackend;
pub use error::InputError;
pub use guard::Input;
pub use pressed::Held;
pub use types::{AbsolutePosition, Button, Key, PressState};

#[cfg(test)]
mod tests;
