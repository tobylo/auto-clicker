//! The Windows backend: every Win32 call the program makes is in this crate
//! (M5 brief decision 3), each behind a safe function, and nothing outside it
//! uses `unsafe`. It answers whether the process runs elevated, reads the
//! user's display languages, shows a message, claims the daemon's single
//! instance and runs the message thread, which hears the session's and the
//! power notices; input, the tray and the rest arrive with the later M5
//! slices.

// Windows alone; on other systems the crate is empty.
#![cfg(windows)]

mod elevation;
mod instance;
mod language;
mod message;
mod message_thread;
mod output;
mod run_timer;
mod session;

pub use elevation::is_elevated;
pub use instance::{ClaimError, Instance, claim};
pub use language::display_languages;
pub use message::show_error;
pub use message_thread::{JoinError, MessageThread, Remote, SessionChange, WindowEvent};
pub use output::{MARKER, Output, received_key};
pub use session::SessionState;
