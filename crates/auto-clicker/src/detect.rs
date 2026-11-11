//! Picking the routes at start, one for each job: where clicks go and where
//! hotkeys come from.
//!
//! An explicit route in the config is used as written. `auto` takes the
//! kernel where the program has access and the desktop's portal otherwise.
//! Only a portal that is absent or too old sends `auto` back to the kernel;
//! one that did not answer in time is kept, since the portal route tries
//! again by itself and the kernel route cannot work without access.
//!
//! Every check is bounded: opening `/dev/uinput` and looking at the event
//! nodes do not block, and each portal call has its own deadline. Detection
//! runs before the daemon handles signals, so a SIGTERM meanwhile ends the
//! process at once; the kernel then destroys any virtual device it made.

use std::future::Future;

use auto_clicker_backend_kernel::{KernelBackend, keyboard_access};
use auto_clicker_backend_portal::{PortalOutput, shortcuts_offered};
use auto_clicker_core::config::{HotkeyRoute, InputSettings, OutputRoute};
use auto_clicker_core::controller::{InputReader, Route, Routes};
use auto_clicker_core::hotkeys::{Bindings, HotkeyStatus, Unavailable};
use auto_clicker_core::input::{Input, InputError};

/// The route clicks take, with what the check found.
#[derive(Debug)]
pub enum Output<K> {
    /// `/dev/uinput`: the devices detection opened, kept for the clicker,
    /// or why they could not be opened.
    Kernel(Result<K, InputError>),
    /// The desktop's remote input, and what its check found wrong, if
    /// anything.
    Portal(Option<InputError>),
}

/// The routes the daemon runs on.
pub struct Detected {
    pub output: Output<Input<KernelBackend>>,
    pub hotkeys: InputReader,
}

/// Picks both routes for `settings`. Clicks come first: the hotkey check
/// leaves out the virtual devices the output check made.
pub async fn detect(
    settings: InputSettings,
    bindings: Bindings,
    portal: &PortalOutput,
) -> Detected {
    let output = choose_output(settings.output, KernelBackend::open, || portal.probe()).await;
    let own = match &output {
        Output::Kernel(Ok(input)) => input.backend().devices().to_vec(),
        _ => Vec::new(),
    };
    let keyboards = || keyboard_access(bindings, &own);
    let hotkeys = choose_hotkeys(settings.hotkeys, keyboards, shortcuts_offered).await;
    let detected = Detected { output, hotkeys };
    eprintln!("auto-clicker: {}", detected.routes().line());
    detected
}

impl Detected {
    /// The routes picked, as the tray names them.
    pub fn routes(&self) -> Routes {
        Routes {
            output: match self.output {
                Output::Kernel(_) => Route::Kernel,
                Output::Portal(_) => Route::Portal,
            },
            hotkeys: self.hotkeys.into(),
        }
    }
}

/// The route for clicks: `open` makes the kernel's devices, `probe` checks
/// the portal without a dialog. Each runs only when the route needs it.
pub async fn choose_output<K, F>(
    route: OutputRoute,
    open: impl FnOnce() -> Result<K, InputError>,
    probe: impl FnOnce() -> F,
) -> Output<K>
where
    F: Future<Output = Result<(), InputError>>,
{
    match route {
        OutputRoute::Kernel => Output::Kernel(open()),
        OutputRoute::Portal => Output::Portal(probe().await.err()),
        OutputRoute::Auto => match open() {
            Err(denied) if no_access(&denied) => match probe().await {
                Ok(()) => Output::Portal(None),
                Err(InputError::PortalUnavailable(why)) => {
                    eprintln!("auto-clicker: the remote input portal cannot be used either: {why}");
                    Output::Kernel(Err(denied))
                }
                Err(slow) => Output::Portal(Some(slow)),
            },
            opened => Output::Kernel(opened),
        },
    }
}

/// Whether opening `/dev/uinput` failed for want of access, which the
/// portal route does without, rather than for a reason a retry may clear.
fn no_access(err: &InputError) -> bool {
    matches!(
        err,
        InputError::NoPermission(_) | InputError::UinputMissing(_)
    )
}

/// The route for hotkeys: `keyboards` looks at the keyboards, `offered`
/// checks the desktop's shortcuts. Each runs only when the route needs it.
///
/// Only keyboards that may not be read send `auto` to the desktop: with no
/// keyboard plugged in yet, the kernel's reader waits for one.
pub async fn choose_hotkeys<F>(
    route: HotkeyRoute,
    keyboards: impl FnOnce() -> HotkeyStatus,
    offered: impl FnOnce() -> F,
) -> InputReader
where
    F: Future<Output = Result<(), InputError>>,
{
    match route {
        HotkeyRoute::Kernel => InputReader::Devices,
        HotkeyRoute::Portal => InputReader::Desktop,
        HotkeyRoute::Auto => {
            let denied = HotkeyStatus::Unavailable(Unavailable::NoPermission);
            if keyboards() != denied {
                return InputReader::Devices;
            }
            match offered().await {
                Err(InputError::PortalUnavailable(why)) => {
                    eprintln!("auto-clicker: the desktop's shortcuts cannot be used either: {why}");
                    InputReader::Devices
                }
                _ => InputReader::Desktop,
            }
        }
    }
}

#[cfg(test)]
mod tests;
