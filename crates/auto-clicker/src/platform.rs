//! The controller's platforms: uinput devices or the desktop's portal for
//! output, and on both the evdev input reader (portal hotkeys are P02's) and
//! the clocks and directory recordings need.

use std::future::pending;
use std::path::PathBuf;
use std::time::Duration;

use auto_clicker_backend_kernel::{
    DeviceIdentity, InputSources, KernelBackend, local_time, monotonic_now, read_input,
};
use auto_clicker_backend_portal::{PortalBackend, PortalOutput};
use auto_clicker_core::controller::{
    HotkeyReader, HotkeySender, InputLifetime, InputLoss, OpenInput, Platform,
};
use auto_clicker_core::hotkeys::Bindings;
use auto_clicker_core::input::Input;
use auto_clicker_core::recording::{LocalTime, RecordingFeed, recordings_dir};

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
        evdev_reader(self.bindings, own, send, feed)
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

    fn event_clock(&self) -> Duration {
        monotonic_now()
    }

    fn local_time(&self) -> Option<LocalTime> {
        local_time()
    }

    fn recordings_dir(&self) -> Option<PathBuf> {
        recordings_dir()
    }

    /// libei's devices live inside the compositor and have no evdev node,
    /// so the reader has none of its own to ignore.
    fn read_input(
        &mut self,
        _own: Option<&Input<PortalBackend>>,
        send: HotkeySender,
        feed: RecordingFeed,
    ) -> HotkeyReader {
        evdev_reader(self.bindings, Vec::new(), send, feed)
    }
}

/// The evdev input reader, ignoring the devices `own`.
fn evdev_reader(
    bindings: Bindings,
    own: Vec<DeviceIdentity>,
    send: HotkeySender,
    feed: RecordingFeed,
) -> HotkeyReader {
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_clocks_and_directory_are_the_backends_and_cores() {
        let kernel = Kernel {
            bindings: Bindings::default(),
        };
        let portal = Portal {
            output: PortalOutput::new(None),
            bindings: Bindings::default(),
        };
        for (clock, local, dir) in [
            (
                kernel.event_clock(),
                kernel.local_time(),
                kernel.recordings_dir(),
            ),
            (
                portal.event_clock(),
                portal.local_time(),
                portal.recordings_dir(),
            ),
        ] {
            let gap = clock.abs_diff(monotonic_now());
            assert!(gap < Duration::from_secs(1), "{gap:?}");
            assert!(local.is_some());
            assert_eq!(dir, recordings_dir());
        }
    }
}
