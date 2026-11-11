//! Saving the config: the file as the daemon last read or wrote it, so a
//! save can tell whether something else changed it since, and the write.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::read::read_config;
use super::{Config, render};
use crate::fs::atomic_write;

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
    /// not be used, is left as it is, and the error says which.
    pub fn save(&mut self, config: &Config, replace: bool) -> Result<(), SaveError> {
        let path = self.path.as_deref().ok_or(SaveError::NoPath)?;
        if !replace && let Some(stale) = self.stale(path) {
            return Err(SaveError::Stale(stale));
        }
        let text = render(config);
        write(path, text.as_bytes()).map_err(SaveError::Write)?;
        self.seen = Seen::Text(text.into_bytes());
        Ok(())
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

/// Writes the file, creating its directory first if it is missing.
fn write(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_write(path, contents)
}
