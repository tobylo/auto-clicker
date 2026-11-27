//! The Windows backend: every Win32 call the program makes is in this crate
//! (M5 brief decision 3), each behind a safe function, and nothing outside it
//! uses `unsafe`. It answers whether the process runs elevated, reads the
//! user's display languages, shows a message, claims the daemon's single
//! instance and runs the message thread, which hears the session's and the
//! power notices and shows the tray icon, registers the hotkeys and reads the
//! keyboard while they need it, sends input, and keeps the table the
//! watchdog releases from should the daemon die, sending again what Windows
//! blocked until it arrives; recording and the rest
//! arrive with the later M5 slices.

// Windows alone; on other systems the crate is empty.
#![cfg(windows)]

mod delivery;
#[cfg(any(test, feature = "desktop"))]
pub mod desktop;
mod dpi;
mod elevation;
mod hotkeys;
mod instance;
mod language;
mod message;
mod message_thread;
mod output;
mod resend;
mod run_timer;
mod screens;
mod session;
mod tray;
pub mod watchdog;
mod watched;

pub use delivery::{Context, Delivery, foreground_takes_input};
pub use dpi::become_per_monitor_aware;
pub use elevation::is_elevated;
pub use hotkeys::{HotkeyReading, Sink};
pub use instance::{ClaimError, Instance, claim};
pub use language::display_languages;
pub use message::show_error;
pub use message_thread::{JoinError, MessageThread, Remote, SessionChange, WindowEvent};
pub use output::{MARKER, Output, received_key};
pub use session::SessionState;
pub use tray::{MenuEntry, TrayContent, TrayIcon, icon_side};
pub use watched::{Desktop, Probe, Watched};
