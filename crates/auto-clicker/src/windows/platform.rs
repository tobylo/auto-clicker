//! The controller's platform on Windows. Output waits for the watchdog: the
//! daemon sends no press before a watchdog is attached to release what it
//! holds should it die (M5 brief decision 6), and the watchdog arrives with
//! WN06, which gives this platform WN04's output in place of [`NoOutput`].
//! Hotkeys arrive with WN08, recording with WN09.

use std::future::pending;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use auto_clicker_core::controller::{HotkeyReader, InputLifetime, OpenInput, Platform, ReadInput};
use auto_clicker_core::input::{
    AbsolutePosition, Button, Input, InputBackend, InputError, Key, PressState, RunTimer,
};
use auto_clicker_core::recording::{LocalTime, recordings_dir};

/// The daemon's platform on Windows.
pub struct Windows {
    /// When the daemon started: the event clock counts from here.
    started: Instant,
}

impl Windows {
    pub fn new() -> Windows {
        Windows {
            started: Instant::now(),
        }
    }
}

/// No output: it cannot be opened, so no value of it exists, and no press
/// is ever sent through it.
pub enum NoOutput {}

impl InputBackend for NoOutput {
    fn button(&mut self, _: Button, _: PressState) -> Result<(), InputError> {
        match *self {}
    }

    fn key(&mut self, _: Key, _: PressState) -> Result<(), InputError> {
        match *self {}
    }

    fn move_relative(&mut self, _: i32, _: i32) -> Result<(), InputError> {
        match *self {}
    }

    fn move_absolute(&mut self, _: AbsolutePosition) -> Result<(), InputError> {
        match *self {}
    }

    fn scroll(&mut self, _: i32) -> Result<(), InputError> {
        match *self {}
    }

    fn run_timer(&mut self) -> Result<Box<dyn RunTimer>, InputError> {
        match *self {}
    }
}

impl Platform for Windows {
    type Backend = NoOutput;

    /// Output is opened for each run, as SendInput needs no device.
    const LIFETIME: InputLifetime = InputLifetime::PerRun;

    fn open_input(&mut self) -> OpenInput<NoOutput> {
        Box::pin(async {
            Err(InputError::CreateDevice(io::Error::new(
                io::ErrorKind::Unsupported,
                "clicking on Windows waits for the watchdog, which is not built yet",
            )))
        })
    }

    /// No hotkeys yet: a reader that never fires.
    fn read_input(&mut self, _own: Option<&Input<NoOutput>>, _reading: ReadInput) -> HotkeyReader {
        Box::pin(pending())
    }

    fn event_clock(&self) -> Duration {
        self.started.elapsed()
    }

    /// Recordings arrive with WN09, which names them by the local time.
    fn local_time(&self) -> Option<LocalTime> {
        None
    }

    fn recordings_dir(&self) -> Option<PathBuf> {
        recordings_dir()
    }
}
