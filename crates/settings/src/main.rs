//! The settings window of auto-clicker, started by the daemon from its tray.
//! Each system has its own module; this file only picks it.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as system;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as system;

fn main() -> std::process::ExitCode {
    system::main()
}
