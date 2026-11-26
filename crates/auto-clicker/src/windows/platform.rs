//! The controller's platform on Windows. Output is SendInput's, keeping the
//! table the watchdog releases from should the daemon die (M5 brief
//! decision 6): the daemon runs only with a watchdog attached, so no press
//! is ever sent without one. Hotkeys arrive with WN08, recording with WN09.

use std::future::pending;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use auto_clicker_backend_windows::Watched;
use auto_clicker_backend_windows::watchdog::Table;
use auto_clicker_core::controller::{HotkeyReader, InputLifetime, OpenInput, Platform, ReadInput};
use auto_clicker_core::input::Input;
use auto_clicker_core::recording::{LocalTime, recordings_dir};

/// The daemon's platform on Windows.
pub struct Windows {
    /// When the daemon started: the event clock counts from here.
    started: Instant,
    /// The table shared with the watchdog.
    table: Arc<Table>,
}

impl Windows {
    pub fn new(table: Table) -> Windows {
        Windows {
            started: Instant::now(),
            table: Arc::new(table),
        }
    }
}

impl Platform for Windows {
    type Backend = Watched;

    /// Output is opened for each run, as SendInput needs no device.
    const LIFETIME: InputLifetime = InputLifetime::PerRun;

    fn open_input(&mut self) -> OpenInput<Watched> {
        let table = Arc::clone(&self.table);
        Box::pin(async move { Ok(Watched::open(table)) })
    }

    /// No hotkeys yet: a reader that never fires.
    fn read_input(&mut self, _own: Option<&Input<Watched>>, _reading: ReadInput) -> HotkeyReader {
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
