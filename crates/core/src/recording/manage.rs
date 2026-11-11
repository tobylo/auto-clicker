//! Changing the recordings the user keeps: renaming one, which rewrites its
//! file whole under the same file name, and deleting one.
//!
//! Each opens the recordings folder once and works through that open
//! folder, like a save, so a folder renamed or replaced meanwhile is never
//! changed. A rename writes the new file beside the old one, flushes it and
//! renames it over the old one, so a crash or a failure leaves the old file
//! whole.

use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::Path;

use super::library::{FileName, LoadError, load_in};
use super::store::{held_path, named, open_folder};
use super::{InvalidRecording, check_name};
use crate::fs::{Place, atomic_save_with};

/// Why a recording was not changed. Nothing was.
#[derive(Debug)]
pub enum ChangeError {
    /// There is no such file, or no recordings folder.
    Missing,
    /// The new name is not one a recording may have.
    Refused(InvalidRecording),
    /// The file could not be read, or is not a valid recording.
    Unreadable(LoadError),
    /// Writing or removing failed, or the folder could not be opened.
    Failed(io::Error),
}

impl fmt::Display for ChangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeError::Missing => f.write_str("the file is no longer there"),
            ChangeError::Refused(err) => write!(f, "{err}"),
            ChangeError::Unreadable(err) => write!(f, "{err}"),
            ChangeError::Failed(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ChangeError {}

/// Renames the recording `file` in `dir` to `name`: reads and checks it
/// whole, and replaces it with the same recording under the new name, in
/// the same file. The name is checked before the file is read.
///
/// A replacement that is in place but whose folder could not be flushed
/// counts as done: a power loss could bring back the old file, whole.
///
/// # Errors
/// See [`ChangeError`].
pub fn rename(dir: &Path, file: &FileName, name: String) -> Result<(), ChangeError> {
    check_name(&name).map_err(ChangeError::Refused)?;
    let folder = open(dir)?;
    let recording = load_in(&folder, file).map_err(|err| match err {
        LoadError::Missing => ChangeError::Missing,
        other => ChangeError::Unreadable(other),
    })?;
    let renamed = recording.renamed(name).map_err(ChangeError::Refused)?;
    let path = held_path(&folder).join(file.as_str());
    // In place but not flushed: done, as the doc says.
    let _unsynced = atomic_save_with(&path, |out| renamed.write_json(out), Place::Replace)
        .map_err(|err| failed(&folder, dir, "written", &err))?;
    Ok(())
}

/// Deletes the recording `file` in `dir`, for good.
///
/// # Errors
/// [`ChangeError::Missing`] when it is already gone; [`ChangeError::Failed`]
/// when it cannot be removed.
pub fn delete(dir: &Path, file: &FileName) -> Result<(), ChangeError> {
    let folder = open(dir)?;
    match fs::remove_file(held_path(&folder).join(file.as_str())) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(ChangeError::Missing),
        Err(err) => return Err(failed(&folder, dir, "deleted", &err)),
    }
    // The file is gone. Should flushing the folder fail, a power loss could
    // bring it back whole, which is no worse than not deleting it.
    let _ = folder.sync_all();
    Ok(())
}

/// Opens the recordings folder, refusing a symlink in its place.
fn open(dir: &Path) -> Result<File, ChangeError> {
    open_folder(dir).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => ChangeError::Missing,
        _ => ChangeError::Failed(err),
    })
}

/// `err`, from a file that could not be `done`, naming the folder by its
/// path rather than by the handle it was reached through.
fn failed(folder: &File, dir: &Path, done: &str, err: &io::Error) -> ChangeError {
    let err = named(folder, dir, err);
    ChangeError::Failed(io::Error::new(
        err.kind(),
        format!("the file could not be {done}: {err}"),
    ))
}

#[cfg(test)]
mod tests;
