//! The Windows backend: every Win32 call the program makes is in this crate
//! (M5 brief decision 3), each behind a safe function, and nothing outside it
//! uses `unsafe`. For now it answers whether the process runs elevated, reads
//! the user's display languages and shows a message; input, the message
//! thread, the tray and the rest arrive with the later M5 slices.

// Windows alone; on other systems the crate is empty.
#![cfg(windows)]

mod elevation;
mod language;
mod message;

pub use elevation::is_elevated;
pub use language::display_languages;
pub use message::show_error;
