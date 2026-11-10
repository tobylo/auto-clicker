//! The reader's side of a recording: opening what a recording reads when
//! one starts, and closing it again when it stops.

use std::io;

use auto_clicker_core::hotkeys::{DeviceId, HotkeyEvent};
use auto_clicker_core::recording::Recorder;

use super::Reader;

impl<S: FnMut(HotkeyEvent)> Reader<S> {
    /// A recording started or stopped. On a start, every node not wanted
    /// before is looked at again (mice and other keyboards are wanted now),
    /// and what every device holds is read for the new recorder. On a stop,
    /// the devices read only for the recording are closed, so idle costs
    /// nothing again.
    pub(super) fn recording_changed(&mut self) -> io::Result<()> {
        if !self.feed.recording() {
            self.close_recording_devices();
            return Ok(());
        }
        // The devices already open first: opening the others can take a
        // while, and what is held now counts from the start.
        let open: Vec<DeviceId> = self
            .devices
            .iter()
            .filter(|device| device.role.record)
            .map(|device| device.id)
            .collect();
        for id in open {
            self.read_state(id);
        }
        self.registry.forget_not_wanted();
        // Devices it opens are read as they are added.
        self.scan()?;
        self.feed.with(Recorder::start_read);
        Ok(())
    }

    fn close_recording_devices(&mut self) {
        let recording_only: Vec<DeviceId> = self
            .devices
            .iter()
            .filter(|device| !device.role.hotkeys)
            .map(|device| device.id)
            .collect();
        for id in recording_only {
            if let Some(name) = self.remove(id) {
                self.registry.forget(&name);
            }
        }
    }
}
