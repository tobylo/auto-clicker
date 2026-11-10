//! The controller's platform on the kernel backend: uinput devices and the
//! evdev hotkey reader.

use std::future::pending;

use auto_clicker_backend_kernel::{DeviceIdentity, KernelBackend, watch_hotkeys};
use auto_clicker_core::controller::{HotkeyReader, HotkeySender, Platform};
use auto_clicker_core::hotkeys::Bindings;
use auto_clicker_core::input::{Input, InputError};

pub struct Kernel {
    pub bindings: Bindings,
}

impl Platform for Kernel {
    type Backend = KernelBackend;

    fn open_input(&mut self) -> Result<Input<KernelBackend>, InputError> {
        KernelBackend::open()
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
        let bindings = self.bindings;
        // The controller stops the reader by dropping it, so it needs no
        // shutdown signal of its own.
        Box::pin(async move {
            watch_hotkeys(bindings, &own, |event| send.send(event), pending()).await;
        })
    }
}
