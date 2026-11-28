//! How the settings window starts on Windows: with two pipes named on its
//! command line and no standard handles (M5 brief decision 14). The keeper
//! is the daemon's own ([`crate::window`]).

use std::io;
use std::path::Path;

use auto_clicker_backend_windows::settings_window::{self, let_forward};
use auto_clicker_core::controller::WindowOpener;
use tokio::sync::mpsc;

use crate::window::{self, Started};

/// Starts the keeper of the settings window; each `()` sent to what it
/// returns is a choice of Settings in the tray. Called on the runtime.
pub fn keep(opener: WindowOpener) -> mpsc::UnboundedSender<()> {
    window::keep(opener, start)
}

/// Starts the window with its pipes. The window is killed when its
/// process is dropped, and allowed to the front before it is asked there.
fn start(program: &Path) -> io::Result<Started> {
    let started = settings_window::start(program)?;
    let id = started.window.id();
    Ok(Started {
        process: Box::new(started.window),
        read: Box::new(started.requests),
        write: Box::new(started.updates),
        let_forward: Box::new(move || let_forward(id)),
    })
}
