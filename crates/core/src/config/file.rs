//! Saving the config: the file as the daemon last read or wrote it, so a
//! save can tell whether something else changed it since, and the write.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::read::read_config;
use super::{Config, render};
use crate::fs::{Place, Unsynced, atomic_save};

/// The config file and what the daemon last saw in it.
#[derive(Debug)]
pub struct ConfigFile {
    /// `None` when there is no config directory.
    path: Option<PathBuf>,
    seen: Seen,
}

/// What the file held when the daemon last read or wrote it.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Seen {
    Missing,
    Text(Vec<u8>),
    /// It could not be read, or did not validate: the daemon ran on the
    /// defaults, and nothing of the file is in its settings.
    Unusable,
}

/// Why a save needs the user's word before it replaces the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stale {
    /// Something else changed the file since the daemon read it.
    Changed,
    /// The file could not be used at start (the settings shown are the
    /// defaults), or cannot be read now.
    Unusable,
}

/// A save that wrote the file.
#[derive(Debug)]
pub enum Saved {
    /// Written and flushed to disk.
    Synced,
    /// Written, but flushing its directory failed: the file holds the new
    /// settings, which may not survive a power loss.
    Unsynced(io::Error),
}

/// Why a save did not write.
#[derive(Debug)]
pub enum SaveError {
    /// The file needs the user's word first.
    Stale(Stale),
    /// There is no config directory.
    NoPath,
    /// Writing failed; the file is as it was.
    Write(io::Error),
}

impl ConfigFile {
    pub(super) fn new(path: &Path, seen: Seen) -> ConfigFile {
        ConfigFile {
            path: Some(path.to_owned()),
            seen,
        }
    }

    /// No config directory: nothing can be saved.
    pub fn none() -> ConfigFile {
        ConfigFile {
            path: None,
            seen: Seen::Unusable,
        }
    }

    /// Writes `config` to the file, atomically. Unless `replace` is set, a
    /// file that changed since it was last read or written, or that could
    /// not be used, is left as it is, and the error says which. A file
    /// that was missing is created only where none appeared since.
    pub fn save(&mut self, config: &Config, replace: bool) -> Result<Saved, SaveError> {
        let path = self.path.as_deref().ok_or(SaveError::NoPath)?;
        if !replace && let Some(stale) = self.stale(path) {
            return Err(SaveError::Stale(stale));
        }
        let place = placement(&self.seen, replace);
        let text = render(config);
        let written = write(path, text.as_bytes(), place).map_err(|err| write_error(err, place))?;
        // In place, flushed or not: the file holds this now.
        self.seen = Seen::Text(text.into_bytes());
        Ok(match written {
            Ok(()) => Saved::Synced,
            Err(unsynced) => Saved::Unsynced(unsynced.into_error()),
        })
    }

    /// Why the file is not as the daemon last saw it, if it is not.
    fn stale(&self, path: &Path) -> Option<Stale> {
        let now = match read_config(path) {
            Ok(None) => Seen::Missing,
            Ok(Some(bytes)) => Seen::Text(bytes),
            Err(_) => return Some(Stale::Unusable),
        };
        match &self.seen {
            Seen::Unusable => Some(Stale::Unusable),
            seen if *seen == now => None,
            _ => Some(Stale::Changed),
        }
    }
}

/// How a save puts the file in place: where the file was missing and the
/// user did not agree to replace one, only where none appeared since.
fn placement(seen: &Seen, replace: bool) -> Place {
    match seen {
        Seen::Missing if !replace => Place::New,
        _ => Place::Replace,
    }
}

/// Why a write did not happen: a file appeared where none was may not be
/// replaced without the user's word.
fn write_error(err: io::Error, place: Place) -> SaveError {
    if place == Place::New && err.kind() == io::ErrorKind::AlreadyExists {
        SaveError::Stale(Stale::Changed)
    } else {
        SaveError::Write(err)
    }
}

/// Writes the file, creating its directory first if it is missing.
fn write(path: &Path, contents: &[u8], place: Place) -> io::Result<Result<(), Unsynced>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_save(path, contents, place)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_missing_file_not_agreed_to_replace_is_placed_as_new() {
        let cases = [
            (Seen::Missing, false, Place::New),
            (Seen::Missing, true, Place::Replace),
            (Seen::Text(Vec::new()), false, Place::Replace),
            (Seen::Unusable, true, Place::Replace),
        ];
        for (seen, replace, place) in cases {
            assert_eq!(placement(&seen, replace), place, "{seen:?} {replace}");
        }
    }

    #[test]
    fn a_file_that_appeared_is_stale_and_any_other_failure_a_write_error() {
        let exists = || io::Error::from(io::ErrorKind::AlreadyExists);
        let denied = || io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(matches!(
            write_error(exists(), Place::New),
            SaveError::Stale(Stale::Changed)
        ));
        assert!(matches!(
            write_error(exists(), Place::Replace),
            SaveError::Write(_)
        ));
        assert!(matches!(
            write_error(denied(), Place::New),
            SaveError::Write(_)
        ));
    }
}
