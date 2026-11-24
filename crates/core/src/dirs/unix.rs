//! The XDG base directories, with their fallbacks under `HOME`.

use std::ffi::OsString;
use std::path::PathBuf;

use super::program_dir;

pub(super) fn config_dir() -> Option<PathBuf> {
    from(var("XDG_CONFIG_HOME"), var("HOME"), &[".config"])
}

pub(super) fn data_dir() -> Option<PathBuf> {
    from(var("XDG_DATA_HOME"), var("HOME"), &[".local", "share"])
}

pub(super) fn state_dir() -> Option<PathBuf> {
    from(var("XDG_STATE_HOME"), var("HOME"), &[".local", "state"])
}

fn var(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// The program's folder in the XDG directory `xdg`, or when it is unset or
/// not absolute, in `home` joined with `fallback`.
fn from(xdg: Option<OsString>, home: Option<OsString>, fallback: &[&str]) -> Option<PathBuf> {
    xdg.map(PathBuf::from).and_then(program_dir).or_else(|| {
        let home = PathBuf::from(home?);
        program_dir(fallback.iter().fold(home, |dir, part| dir.join(part)))
    })
}

#[cfg(test)]
mod tests;
