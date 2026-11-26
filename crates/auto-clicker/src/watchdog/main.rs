//! The watchdog (M5 brief decision 6): what starts at login on Windows. It
//! starts the daemon as its child, shares the table of what the daemon
//! holds, sleeps on the daemon's process, and once the daemon has ended,
//! for whatever reason, releases what the table still holds and exits with
//! the daemon's exit code. Being the parent, it outlives "End process tree"
//! on the daemon, and a kill by the daemon's name.
//!
//! Windows alone: elsewhere it says so and exits.

// No console window opens on Windows: the watchdog writes to its log file.
#![cfg_attr(not(test), windows_subsystem = "windows")]

// The daemon's own elevation check, shared.
#[cfg(windows)]
#[path = "../windows/elevation.rs"]
mod elevation;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    windows::main()
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("auto-clicker-watchdog runs on Windows only");
    std::process::ExitCode::from(2)
}
