//! Changing the recordings the user keeps: renaming one, which rewrites its
//! file whole under the same file name, and deleting one.
//!
//! Each opens the recordings folder once and works through that open
//! folder, like a save, so a folder renamed or replaced meanwhile is never
//! changed. A rename writes the new file beside the old one, flushes it and
//! renames it over the old one, so a crash or a failure leaves the old file
//! whole.

use std::cell::Cell;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use super::library::{FileName, LoadError, MAX_FILE_BYTES, load_in};
use super::store::{held_path, named, open_folder};
use super::{InvalidRecording, check_name};
use crate::fs::{Place, atomic_save_with};
use crate::message::{ChangeFailure, Quoted, Step, Unplayable};

/// Why a recording was not changed. Nothing was.
#[derive(Debug)]
pub enum ChangeError {
    /// There is no such file, or no recordings folder.
    Missing,
    /// The new name is not one a recording may have.
    Refused(InvalidRecording),
    /// The file could not be read, or is not a valid recording.
    Unreadable(LoadError),
    /// The folder could not be opened.
    Failed(io::Error),
    /// Writing or removing the file failed.
    NotDone { step: Step, err: io::Error },
    /// The renamed recording would be larger than a load reads.
    TooLarge,
}

impl From<&ChangeError> for ChangeFailure {
    fn from(err: &ChangeError) -> ChangeFailure {
        let quoted = |err: &dyn fmt::Display| Quoted::new(err.to_string());
        match err {
            ChangeError::Missing => ChangeFailure::Missing,
            ChangeError::Refused(why) => ChangeFailure::Refused(quoted(why)),
            ChangeError::Unreadable(why) => ChangeFailure::Unreadable(Unplayable::from(why)),
            ChangeError::Failed(why) => ChangeFailure::Folder(quoted(why)),
            ChangeError::NotDone { step, err } => ChangeFailure::NotDone {
                step: *step,
                why: quoted(err),
            },
            ChangeError::TooLarge => ChangeFailure::TooLarge {
                mib: MAX_FILE_BYTES / (1024 * 1024),
            },
        }
    }
}

/// The same words the window shows, in English: see [`ChangeFailure`].
impl fmt::Display for ChangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        ChangeFailure::from(self).fmt(f)
    }
}

impl std::error::Error for ChangeError {}

/// A change that was made: on disk, or in place but with its folder not
/// flushed, so a power loss could undo it, whole.
#[derive(Debug)]
pub enum Changed {
    Synced,
    Unsynced(io::Error),
}

/// Renames the recording `file` in `dir` to `name`: reads and checks it
/// whole, and replaces it with the same recording under the new name, in
/// the same file. The name is checked before the file is read. The new
/// file may be no larger than a load reads ([`MAX_FILE_BYTES`]), or the
/// recording could no longer be played.
///
/// # Errors
/// See [`ChangeError`].
pub fn rename(dir: &Path, file: &FileName, name: String) -> Result<Changed, ChangeError> {
    rename_capped(dir, file, name, MAX_FILE_BYTES)
}

/// [`rename`], writing at most `cap` bytes.
fn rename_capped(
    dir: &Path,
    file: &FileName,
    name: String,
    cap: u64,
) -> Result<Changed, ChangeError> {
    check_name(&name).map_err(ChangeError::Refused)?;
    let folder = open(dir)?;
    let recording = load_in(&folder, file).map_err(|err| match err {
        LoadError::Missing => ChangeError::Missing,
        other => ChangeError::Unreadable(other),
    })?;
    let renamed = recording.renamed(name).map_err(ChangeError::Refused)?;
    let path = held_path(&folder).join(file.as_str());
    let over = Cell::new(false);
    let write = |out: &mut dyn Write| {
        let over = &over;
        renamed.write_json(&mut Capped {
            out,
            left: cap,
            over,
        })
    };
    match atomic_save_with(&path, write, Place::Replace) {
        Ok(Ok(())) => Ok(Changed::Synced),
        Ok(Err(unsynced)) => Ok(Changed::Unsynced(named(
            &folder,
            dir,
            &unsynced.into_error(),
        ))),
        Err(err) if too_large(over.get(), &err) => Err(ChangeError::TooLarge),
        Err(err) => Err(failed(&folder, dir, Step::Written, &err)),
    }
}

/// Whether a failed write failed for the cap alone: the cap was reached
/// (`over`), and `err` is still the cap's own bare error. A temporary file
/// that could not be removed after it turns the error into one that names
/// that file, which is reported as it is.
fn too_large(over: bool, err: &io::Error) -> bool {
    over && err.kind() == io::ErrorKind::FileTooLarge && err.get_ref().is_none()
}

/// A writer that refuses to take more than `left` bytes more, and notes in
/// `over` that it did.
struct Capped<'a> {
    out: &'a mut dyn Write,
    left: u64,
    over: &'a Cell<bool>,
}

impl Write for Capped<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let fits = u64::try_from(bytes.len()).is_ok_and(|len| len <= self.left);
        if !fits {
            self.over.set(true);
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        let written = self.out.write(bytes)?;
        self.left = self
            .left
            .saturating_sub(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// Deletes the recording `file` in `dir`, for good.
///
/// # Errors
/// [`ChangeError::Missing`] when it is already gone;
/// [`ChangeError::NotDone`] when it cannot be removed.
pub fn delete(dir: &Path, file: &FileName) -> Result<Changed, ChangeError> {
    let folder = open(dir)?;
    match fs::remove_file(held_path(&folder).join(file.as_str())) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(ChangeError::Missing),
        Err(err) => return Err(failed(&folder, dir, Step::Deleted, &err)),
    }
    Ok(match folder.sync_all() {
        Ok(()) => Changed::Synced,
        Err(err) => Changed::Unsynced(err),
    })
}

/// Opens the recordings folder, refusing a symlink in its place.
fn open(dir: &Path) -> Result<File, ChangeError> {
    open_folder(dir).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => ChangeError::Missing,
        _ => ChangeError::Failed(err),
    })
}

/// `err`, from a file that could not be through `step`, naming the folder
/// by its path rather than by the handle it was reached through.
fn failed(folder: &File, dir: &Path, step: Step, err: &io::Error) -> ChangeError {
    ChangeError::NotDone {
        step,
        err: named(folder, dir, err),
    }
}

#[cfg(test)]
mod tests;
