//! The folders Windows names in the environment of every user process:
//! `APPDATA` (roaming, for the config) and `LOCALAPPDATA` (this machine's,
//! for recordings, state and logs).

use std::ffi::OsString;
use std::path::PathBuf;

use super::program_dir;

pub(super) fn config_dir() -> Option<PathBuf> {
    from(std::env::var_os("APPDATA"))
}

pub(super) fn data_dir() -> Option<PathBuf> {
    from(std::env::var_os("LOCALAPPDATA"))
}

pub(super) fn state_dir() -> Option<PathBuf> {
    data_dir()
}

/// The program's folder in `dir`, when it is set and absolute.
fn from(dir: Option<OsString>) -> Option<PathBuf> {
    dir.map(PathBuf::from).and_then(program_dir)
}

#[cfg(test)]
mod tests;
