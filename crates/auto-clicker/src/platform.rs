//! The controller's platform on the kernel backend: uinput devices, the
//! evdev input reader, and the clocks and directory recordings need.

use std::future::pending;
use std::path::PathBuf;
use std::time::Duration;

use auto_clicker_backend_kernel::{
    DeviceIdentity, InputSources, KernelBackend, local_time, monotonic_now, read_input,
};
use auto_clicker_core::controller::{HotkeyReader, HotkeySender, Platform};
use auto_clicker_core::hotkeys::Bindings;
use auto_clicker_core::input::{Input, InputError};
use auto_clicker_core::recording::{LocalTime, RecordingFeed, recordings_dir};

pub struct Kernel {
    pub bindings: Bindings,
}

impl Platform for Kernel {
    type Backend = KernelBackend;

    fn open_input(&mut self) -> Result<Input<KernelBackend>, InputError> {
        KernelBackend::open()
    }

    fn event_clock(&self) -> Duration {
        monotonic_now()
    }

    fn local_time(&self) -> Option<LocalTime> {
        local_time()
    }

    fn recordings_dir(&self) -> Option<PathBuf> {
        recordings_dir()
    }

    fn read_input(
        &mut self,
        own: Option<&Input<KernelBackend>>,
        send: HotkeySender,
        feed: RecordingFeed,
    ) -> HotkeyReader {
        // The reader keeps its own copy: the devices move into each run.
        let own: Vec<DeviceIdentity> = own
            .map(|input| input.backend().devices().to_vec())
            .unwrap_or_default();
        let bindings = self.bindings;
        // The controller stops the reader by dropping it, so it needs no
        // shutdown signal of its own.
        Box::pin(async move {
            let sources = InputSources {
                bindings,
                own: &own,
                feed,
            };
            read_input(sources, |event| send.send(event), pending()).await;
        })
    }
}
