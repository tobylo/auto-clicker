//! Core logic of auto-clicker. Nothing in this crate opens a device, so all of
//! it compiles and tests without `/dev/uinput`, evdev or a compositor.

pub mod input;
