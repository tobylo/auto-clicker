//! The auto-clicker daemon: the tray icon, hotkeys and the clicker, run by one
//! controller (on Windows, beside a Win32 message thread). Each system has
//! its own module; this file only picks it.

// No console window opens on Windows: the daemon writes to its log file.
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod config;
mod tray_icon;
mod tray_menu;
mod tray_text;

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
