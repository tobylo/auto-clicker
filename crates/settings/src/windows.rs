//! The settings window on Windows: not there yet (M5, WN10 builds it).

use std::process::ExitCode;

use auto_clicker_core::log;

/// Says the window is not supported on Windows yet, and fails.
pub fn main() -> ExitCode {
    log::line(format_args!(
        "auto-clicker-settings: the settings window is not supported on Windows yet"
    ));
    ExitCode::FAILURE
}
