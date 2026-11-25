//! The folders the program keeps its files in, each its own `auto-clicker`
//! folder under one the system names: on Linux the XDG base directories
//! (map D7), on Windows `%APPDATA%` and `%LOCALAPPDATA%` (M5 brief decision
//! 12). Each must be an absolute path; otherwise there is none, and never a
//! guessed one.

use std::path::PathBuf;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as system;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as system;

pub use system::{NO_CONFIG_DIR, NO_DATA_DIR};

/// The program's folder for its config: `$XDG_CONFIG_HOME/auto-clicker`
/// (`~/.config` when unset or not absolute), or `%APPDATA%\auto-clicker`.
pub fn config_dir() -> Option<PathBuf> {
    system::config_dir()
}

/// The program's folder for its recordings: `$XDG_DATA_HOME/auto-clicker`
/// (`~/.local/share` when unset or not absolute), or
/// `%LOCALAPPDATA%\auto-clicker`.
pub fn data_dir() -> Option<PathBuf> {
    system::data_dir()
}

/// The program's folder for its state and logs:
/// `$XDG_STATE_HOME/auto-clicker` (`~/.local/state` when unset or not
/// absolute), or `%LOCALAPPDATA%\auto-clicker`.
pub fn state_dir() -> Option<PathBuf> {
    system::state_dir()
}

/// The log file, for a program with no console to write to:
/// `logs/auto-clicker.log` in the state folder.
pub fn log_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("logs").join("auto-clicker.log"))
}

/// `dir`'s `auto-clicker` folder, when `dir` is an absolute path.
fn program_dir(dir: PathBuf) -> Option<PathBuf> {
    dir.is_absolute().then(|| dir.join("auto-clicker"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_variables_that_were_missing() {
        let expected = if cfg!(windows) {
            (
                "APPDATA is not set to an absolute folder",
                "LOCALAPPDATA is not set to an absolute folder",
            )
        } else {
            (
                "neither XDG_CONFIG_HOME nor HOME is set",
                "neither XDG_DATA_HOME nor HOME is set",
            )
        };
        assert_eq!((NO_CONFIG_DIR, NO_DATA_DIR), expected);
    }

    #[test]
    fn each_folder_is_the_systems_and_the_log_is_in_the_state_folder() {
        // The test environment names a home folder (HOME, or APPDATA and
        // LOCALAPPDATA), so each folder exists and a lost one shows.
        assert!(config_dir().is_some() && data_dir().is_some() && state_dir().is_some());
        assert_eq!(config_dir(), system::config_dir());
        assert_eq!(data_dir(), system::data_dir());
        assert_eq!(state_dir(), system::state_dir());
        assert_eq!(
            log_path(),
            system::state_dir().map(|dir| dir.join("logs").join("auto-clicker.log"))
        );
    }
}
