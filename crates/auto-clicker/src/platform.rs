//! The controller's platforms: uinput devices or the desktop's portal for
//! output, and on either the hotkeys from the keyboards or from the
//! desktop's shortcuts, the screen layout from KWin or the Wayland display
//! and the pointer from KWin, and the clocks and directory recordings need.

use std::future::pending;
use std::path::PathBuf;
use std::time::Duration;

use auto_clicker_backend_kernel::{
    DeviceIdentity, InputSources, KernelBackend, local_time, monotonic_now, read_input,
};
use auto_clicker_backend_portal::{PortalBackend, PortalOutput, read_shortcuts};
use auto_clicker_core::controller::{
    HotkeyReader, InputLifetime, InputLoss, InputReader, LayoutSignal, LayoutWatch, OpenInput,
    Platform, ReadInput, Route,
};
use auto_clicker_core::input::Input;
use auto_clicker_core::recording::{Layout, LocalTime, RecordingFeed, recordings_dir};

use crate::screens::Screens;

/// Where hotkeys come from, on either output: the reader detection picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkeys {
    pub reader: InputReader,
}

impl Hotkeys {
    /// Starts the reader. Only the keyboard reader has
    /// devices of the program's `own` to leave out, a recording to feed and
    /// keys to capture; the desktop's is asked for the bindings at its first
    /// bind, and opens the desktop's settings when the controller asks.
    fn read(self, own: Vec<DeviceIdentity>, reading: ReadInput) -> HotkeyReader {
        match self.reader {
            InputReader::Devices => evdev_reader(own, reading),
            // The controller stops the reader by dropping it, which closes
            // the desktop's session.
            InputReader::Desktop => {
                let ReadInput {
                    bindings,
                    send,
                    commands,
                    ..
                } = reading;
                Box::pin(read_shortcuts(bindings, commands, move |event| {
                    send.send(event);
                }))
            }
        }
    }
}

pub struct Kernel {
    pub hotkeys: Hotkeys,
    pub screens: Screens,
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
        reading: ReadInput,
    ) -> HotkeyReader {
        // The reader keeps its own copy: the devices move into each run.
        let own: Vec<DeviceIdentity> = own
            .map(|input| input.backend().devices().to_vec())
            .unwrap_or_default();
        self.hotkeys.read(own, reading)
    }

    fn input_reader(&self) -> InputReader {
        self.hotkeys.reader
    }

    fn layout(&self) -> Option<Layout> {
        self.screens.layout()
    }

    fn watch_layout(&mut self, feed: RecordingFeed, changed: LayoutSignal) -> LayoutWatch {
        self.screens.watch(feed, changed)
    }
}

/// Output through the desktop's RemoteDesktop portal, one session per run.
pub struct Portal {
    pub output: PortalOutput,
    pub hotkeys: Hotkeys,
    pub screens: Screens,
}

impl Platform for Portal {
    type Backend = PortalBackend;

    const LIFETIME: InputLifetime = InputLifetime::PerRun;

    const OUTPUT: Route = Route::Portal;

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

    fn input_reader(&self) -> InputReader {
        self.hotkeys.reader
    }

    /// libei's devices live inside the compositor and have no evdev node,
    /// so the reader has none of its own to ignore.
    fn read_input(
        &mut self,
        _own: Option<&Input<PortalBackend>>,
        reading: ReadInput,
    ) -> HotkeyReader {
        self.hotkeys.read(Vec::new(), reading)
    }

    fn layout(&self) -> Option<Layout> {
        self.screens.layout()
    }

    fn watch_layout(&mut self, feed: RecordingFeed, changed: LayoutSignal) -> LayoutWatch {
        self.screens.watch(feed, changed)
    }
}

/// The evdev input reader, ignoring the devices `own`. It has nothing to
/// open in the desktop, so it takes no command.
fn evdev_reader(own: Vec<DeviceIdentity>, reading: ReadInput) -> HotkeyReader {
    let ReadInput {
        bindings,
        send,
        feed,
        capture,
        ..
    } = reading;
    // The controller stops the reader by dropping it, so it needs no
    // shutdown signal of its own.
    Box::pin(async move {
        let sources = InputSources {
            bindings,
            capture,
            own: &own,
            feed,
        };
        read_input(sources, |event| send.send(event), pending()).await;
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use auto_clicker_backend_kwin::Kwin;
    use auto_clicker_backend_wayland::{Missing, Wayland};

    use super::*;

    fn no_screens() -> Screens {
        Screens::new(Kwin::none(), Wayland::none(Missing::NoDisplay))
    }

    fn hotkeys(reader: InputReader) -> Hotkeys {
        Hotkeys { reader }
    }

    #[test]
    fn the_picked_reader_is_the_readers_on_either_output() {
        for reader in [InputReader::Devices, InputReader::Desktop] {
            let kernel = Kernel {
                hotkeys: hotkeys(reader),
                screens: no_screens(),
            };
            let portal = Portal {
                output: PortalOutput::new(None),
                hotkeys: hotkeys(reader),
                screens: no_screens(),
            };
            assert_eq!(kernel.input_reader(), reader);
            assert_eq!(portal.input_reader(), reader);
        }
        assert_eq!(Kernel::OUTPUT, Route::Kernel);
        assert_eq!(Portal::OUTPUT, Route::Portal);
    }

    #[test]
    fn the_clocks_and_directory_are_the_backends_and_cores() {
        let kernel = Kernel {
            hotkeys: hotkeys(InputReader::Devices),
            screens: no_screens(),
        };
        let portal = Portal {
            output: PortalOutput::new(None),
            hotkeys: hotkeys(InputReader::Desktop),
            screens: no_screens(),
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
