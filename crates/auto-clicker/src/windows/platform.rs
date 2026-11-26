//! The controller's platform on Windows. Output is SendInput's, keeping the
//! table the watchdog releases from should the daemon die (M5 brief
//! decision 6): the daemon runs only with a watchdog attached, so no press
//! is ever sent without one. Hotkeys are registered on the message thread
//! (decision 9); recording arrives with WN09.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use auto_clicker_backend_windows::watchdog::Table;
use auto_clicker_backend_windows::{Remote, Sink, Watched, foreground_takes_input};
use auto_clicker_core::controller::{
    HotkeyReader, InputLifetime, OpenInput, Platform, ReadInput, ReaderCommand,
};
use auto_clicker_core::hotkeys::{HotkeyEvent, HotkeyStatus, Unavailable};
use auto_clicker_core::input::Input;
use auto_clicker_core::recording::{LocalTime, recordings_dir};

/// The daemon's platform on Windows.
pub struct Windows {
    /// When the daemon started: the event clock counts from here.
    started: Instant,
    /// The table shared with the watchdog.
    table: Arc<Table>,
    /// The message thread, which registers the hotkeys; set once it runs,
    /// before the controller does.
    thread: Arc<OnceLock<Remote>>,
}

impl Windows {
    pub fn new(table: Table, thread: Arc<OnceLock<Remote>>) -> Windows {
        Windows {
            started: Instant::now(),
            table: Arc::new(table),
            thread,
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

    /// Windows delivers the hotkeys, and the program's own key presses
    /// would set them off as the user's do.
    fn reports_triggers(&self) -> bool {
        true
    }

    fn foreground_takes_input(&self) -> bool {
        foreground_takes_input()
    }

    /// Registers the hotkeys on the message thread for as long as the
    /// reader runs, and passes on what the controller wants the keyboard
    /// read for.
    fn read_input(&mut self, _own: Option<&Input<Watched>>, reading: ReadInput) -> HotkeyReader {
        let ReadInput {
            bindings,
            send,
            capture,
            mut commands,
            ..
        } = reading;
        let thread = self.thread.get().cloned();
        let own = Arc::clone(&self.table);
        Box::pin(async move {
            let failed = |why: String| {
                let why = Unavailable::ReadFailed(why.into());
                send.send(HotkeyEvent::Status(HotkeyStatus::Unavailable(why)));
            };
            // The settings window that captures keys arrives with WN10.
            if capture.is_some() {
                return failed("keys cannot be captured on Windows yet".to_owned());
            }
            let Some(thread) = thread else {
                return failed("the message thread is not running".to_owned());
            };
            let sink: Sink = {
                let send = send.clone();
                Arc::new(move |event| send.send(event))
            };
            let hotkeys = match thread.read_hotkeys(bindings, own, sink) {
                Ok(hotkeys) => hotkeys,
                Err(err) => return failed(format!("the message thread refused them: {err}")),
            };
            loop {
                if let ReaderCommand::Watch(watching) = commands.next().await {
                    // One that could not be posted is caught up by the
                    // next, or ends with the window.
                    let _ = hotkeys.watch(watching);
                }
            }
        })
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
