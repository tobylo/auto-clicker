//! The folders Windows names in the environment of every user process:
//! `APPDATA` (roaming, for the config) and `LOCALAPPDATA` (this machine's,
//! for recordings, state and logs).

use std::ffi::OsString;
use std::path::PathBuf;

use super::program_dir;

/// Why there is no config folder, as a message says it.
pub const NO_CONFIG_DIR: &str = "APPDATA is not set to an absolute folder";
/// Why there is no data folder, as a message says it.
pub const NO_DATA_DIR: &str = "LOCALAPPDATA is not set to an absolute folder";

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
