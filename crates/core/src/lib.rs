//! Core logic of auto-clicker. Nothing in this crate opens a device, so all of
//! it compiles and tests without `/dev/uinput`, evdev or a compositor.

pub mod clicker;
pub mod config;
pub mod controller;
pub mod fs;
pub mod hotkeys;
pub mod input;
pub mod locale;
pub mod playback;
pub mod recording;
pub mod run;
pub mod window;

#[cfg(test)]
mod test_support;
