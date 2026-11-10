//! The portal input backend: clicks and keys through the desktop's
//! RemoteDesktop portal and libei, without `/dev/uinput`.
//!
//! This crate is the only one that touches the portals and libei. It hands
//! out an [`Input`] over its backend and never the bare backend, so every
//! press goes through the pressed set that releases it.
//!
//! A session lives for one run: [`PortalOutput::open`] starts it (the
//! desktop asks the user the first time; a saved token answers every later
//! start in milliseconds), and dropping the input ends it. While a session
//! is open the desktop shows that remote input is active. When the session
//! ends, the compositor releases whatever its devices still held.

mod eis;
mod layout;
mod session;
mod token;

use std::future::Future;
use std::path::PathBuf;
use std::rc::Rc;

use ashpd::desktop::remote_desktop::DeviceType;
use ashpd::enumflags2::BitFlags;
use auto_clicker_core::input::{
    AbsolutePosition, Button, Input, InputBackend, InputError, Key, PressState,
};
use reis::ei;
use reis::event::DeviceCapability;
use tokio::sync::OnceCell;

pub use session::APP_ID;
pub use token::token_path;

use eis::Eis;
use session::{Portal, SessionGuard};

/// libei's discrete scroll units per wheel notch.
const UNITS_PER_NOTCH: i32 = 120;

/// One run's portal session and its libei devices.
pub struct PortalBackend {
    // Fields drop in order: the connection ends first, which releases what
    // the devices held, then the session closes.
    eis: Eis,
    _session: SessionGuard,
}

impl std::fmt::Debug for PortalBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PortalBackend")
    }
}

/// Opens portal sessions. The registered bus connection is made once and
/// kept: the portal binds the app id to it.
#[derive(Clone, Debug, Default)]
pub struct PortalOutput {
    portal: Rc<OnceCell<Portal>>,
    token_path: Option<PathBuf>,
}

impl PortalOutput {
    /// Sessions that keep their restore token at `token_path` (`None`: the
    /// desktop asks on every start).
    pub fn new(token_path: Option<PathBuf>) -> PortalOutput {
        PortalOutput {
            portal: Rc::default(),
            token_path,
        }
    }

    /// Registers the app id and checks that the portal can serve the
    /// program, without a dialog.
    pub async fn probe(&self) -> Result<(), InputError> {
        self.portal().await?.probe().await
    }

    /// Starts a session and connects to its devices. The future owns what
    /// it opened: dropping it part way (a stop while the dialog shows)
    /// closes the session.
    ///
    /// # Errors
    /// [`InputError::NotAllowed`] when the user did not allow input,
    /// [`InputError::AllowedNow`] when they just did (the next start goes
    /// ahead), [`InputError::PortalUnavailable`] when the desktop cannot
    /// serve the program, [`InputError::Portal`] when a call failed or did
    /// not answer.
    pub fn open(&self) -> impl Future<Output = Result<Input<PortalBackend>, InputError>> + 'static {
        let output = self.clone();
        async move {
            let portal = output.portal().await?;
            let started = portal.start(output.token_path.as_deref()).await?;
            let eis = Eis::connect(started.socket, &capabilities(started.devices)).await?;
            Ok(Input::new(PortalBackend {
                eis,
                _session: started.session,
            }))
        }
    }

    /// Resolves when `input`'s session is lost: the desktop ended it or
    /// paused its devices.
    pub fn lost(input: &Input<PortalBackend>) -> impl Future<Output = InputError> + 'static {
        input.backend().eis.lost()
    }

    async fn portal(&self) -> Result<&Portal, InputError> {
        self.portal.get_or_try_init(Portal::connect).await
    }
}

/// The libei capabilities the allowed device types bring.
fn capabilities(devices: BitFlags<DeviceType>) -> Vec<DeviceCapability> {
    let mut needed = Vec::new();
    if devices.contains(DeviceType::Pointer) {
        needed.extend([
            DeviceCapability::Pointer,
            DeviceCapability::PointerAbsolute,
            DeviceCapability::Button,
            DeviceCapability::Scroll,
        ]);
    }
    if devices.contains(DeviceType::Keyboard) {
        needed.push(DeviceCapability::Keyboard);
    }
    needed
}

impl PortalBackend {
    fn emit<T: ei::Interface>(
        &mut self,
        capability: DeviceCapability,
        what: &str,
        request: impl FnOnce(&T),
    ) -> Result<(), InputError> {
        self.eis.emit(capability, what, request)
    }
}

fn button_state(state: PressState) -> ei::button::ButtonState {
    match state {
        PressState::Down => ei::button::ButtonState::Press,
        PressState::Up => ei::button::ButtonState::Released,
    }
}

fn key_state(state: PressState) -> ei::keyboard::KeyState {
    match state {
        PressState::Down => ei::keyboard::KeyState::Press,
        PressState::Up => ei::keyboard::KeyState::Released,
    }
}

/// The libei request that presses or releases `button`.
fn button_request(button: Button, state: PressState) -> impl FnOnce(&ei::Button) {
    let code = u32::from(button.code());
    move |device| device.button(code, button_state(state))
}

/// The libei request that presses or releases `key`.
fn key_request(key: Key, state: PressState) -> impl FnOnce(&ei::Keyboard) {
    let code = u32::from(key.code());
    move |device| device.key(code, key_state(state))
}

/// libei's discrete scroll for `notches`: positive scrolls down there, up
/// in the trait.
fn scroll_units(notches: i32) -> i32 {
    notches.saturating_mul(-UNITS_PER_NOTCH)
}

/// A relative step in libei's logical pixels. Steps are small; `f32` is
/// exact up to 2^24.
#[allow(clippy::cast_precision_loss)]
fn step(delta: i32) -> f32 {
    delta as f32
}

impl InputBackend for PortalBackend {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        let request = button_request(button, state);
        self.emit(DeviceCapability::Button, "press a mouse button", request)
    }

    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError> {
        self.emit(
            DeviceCapability::Keyboard,
            "press a key",
            key_request(key, state),
        )
    }

    fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        self.emit(
            DeviceCapability::Pointer,
            "move the pointer",
            |p: &ei::Pointer| {
                p.motion_relative(step(dx), step(dy));
            },
        )
    }

    fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError> {
        let (x, y) = self.eis.layout()?.point(position);
        self.emit(
            DeviceCapability::PointerAbsolute,
            "move the pointer to a point",
            |p: &ei::PointerAbsolute| p.motion_absolute(x, y),
        )
    }

    fn scroll(&mut self, notches: i32) -> Result<(), InputError> {
        let units = scroll_units(notches);
        self.emit(DeviceCapability::Scroll, "scroll", |s: &ei::Scroll| {
            s.scroll_discrete(0, units);
        })
    }
}

#[cfg(test)]
mod fake_eis;
#[cfg(test)]
mod plasma_tests;
#[cfg(test)]
mod tests;
