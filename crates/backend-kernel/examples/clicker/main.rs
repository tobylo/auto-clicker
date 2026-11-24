//! Runs the clicker engine on the kernel backend with the settings of a
//! config file, for the click-rate and idle-cost measurements
//! (scripts/measure-clicker.py). Not part of the installed program.
//!
//! ```text
//! cargo run --release --locked -p auto-clicker-backend-kernel --example clicker -- CONFIG [START_DELAY_MS [RUN_FOR_MS]]
//! ```
//!
//! It prints `mouse <event node>` once the virtual devices exist, waits
//! START_DELAY_MS (default 1000) for the compositor to add them, prints
//! `start`, runs until the repeat count, Ctrl+C or SIGTERM, and prints how it
//! ended. With RUN_FOR_MS, it also stops by itself that long after it
//! started, so a run whose caller died cannot click on: the measurement
//! script always passes it. Leave the mouse and keyboard alone while it runs.

// The kernel backend is Linux alone; elsewhere the example does nothing.
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    linux::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {}
