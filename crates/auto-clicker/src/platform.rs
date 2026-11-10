//! The controller's platforms: uinput devices or the desktop's portal for
//! output, and the evdev hotkey reader on both (portal hotkeys are P02's).

use std::future::pending;

use auto_clicker_backend_kernel::{DeviceIdentity, KernelBackend, watch_hotkeys};
use auto_clicker_backend_portal::{PortalBackend, PortalOutput};
use auto_clicker_core::controller::{
    HotkeyReader, HotkeySender, InputLifetime, InputLoss, OpenInput, Platform,
};
use auto_clicker_core::hotkeys::Bindings;
use auto_clicker_core::input::Input;

pub struct Kernel {
    pub bindings: Bindings,
}

impl Platform for Kernel {
    type Backend = KernelBackend;

    const LIFETIME: InputLifetime = InputLifetime::Kept;

    fn open_input(&mut self) -> OpenInput<KernelBackend> {
        let opened = KernelBackend::open();
        Box::pin(async move { opened })
    }

    fn read_hotkeys(
        &mut self,
        own: Option<&Input<KernelBackend>>,
        send: HotkeySender,
    ) -> HotkeyReader {
        // The reader keeps its own copy: the devices move into each run.
        let own: Vec<DeviceIdentity> = own
            .map(|input| input.backend().devices().to_vec())
            .unwrap_or_default();
        read_hotkeys(self.bindings, own, send)
    }
}

/// Output through the desktop's RemoteDesktop portal, one session per run.
pub struct Portal {
    pub output: PortalOutput,
    pub bindings: Bindings,
}

impl Platform for Portal {
    type Backend = PortalBackend;

    const LIFETIME: InputLifetime = InputLifetime::PerRun;

    fn open_input(&mut self) -> OpenInput<PortalBackend> {
        Box::pin(self.output.open())
    }

    fn input_lost(&self, input: &Input<PortalBackend>) -> InputLoss {
        Box::pin(PortalOutput::lost(input))
    }

    /// libei's devices live inside the compositor and have no evdev node,
    /// so the reader has none of its own to ignore.
    fn read_hotkeys(
        &mut self,
        _own: Option<&Input<PortalBackend>>,
        send: HotkeySender,
    ) -> HotkeyReader {
        read_hotkeys(self.bindings, Vec::new(), send)
    }
}

/// The evdev hotkey reader, ignoring the devices `own`.
fn read_hotkeys(bindings: Bindings, own: Vec<DeviceIdentity>, send: HotkeySender) -> HotkeyReader {
    // The controller stops the reader by dropping it, so it needs no
    // shutdown signal of its own.
    Box::pin(async move {
        watch_hotkeys(bindings, &own, |event| send.send(event), pending()).await;
    })
}
