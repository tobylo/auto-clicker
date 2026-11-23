//! Changing the recordings the user keeps: renaming one, which rewrites its
//! file whole under the same file name, and deleting some or all of them.
//!
//! Each opens the recordings folder once and works through that open
//! folder, like a save, so a folder renamed or replaced meanwhile is never
//! changed. A rename writes the new file beside the old one, flushes it and
//! renames it over the old one, so a crash or a failure leaves the old file
//! whole.

use std::cell::Cell;
use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use super::library::{FileName, LoadError, MAX_FILE_BYTES, load_in};
use super::{InvalidRecording, check_name};
use crate::fs::{Folder, Temp};
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
    let over = Cell::new(false);
    let write = |out: &mut dyn Write| {
        let over = &over;
        renamed.write_json(&mut Capped {
            out,
            left: cap,
            over,
        })
    };
    match folder.replace(file.as_str(), write) {
        Ok(Ok(())) => Ok(Changed::Synced),
        Ok(Err(unsynced)) => Ok(Changed::Unsynced(folder.named(dir, &unsynced.into_error()))),
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

/// Which recordings a deletion takes.
#[derive(Clone, Copy, Debug)]
pub enum Doomed<'a> {
    These(&'a [FileName]),
    /// Every file in the folder with a recording's name, as the folder
    /// holds them when the deletion begins.
    All,
}

/// What a deletion did.
#[derive(Debug, Default)]
pub struct Removed {
    /// How many files it removed.
    pub deleted: usize,
    /// The first files it could not remove, and why.
    pub failed: Vec<(FileName, ChangeError)>,
    /// How many more it could not remove.
    pub more_failed: usize,
    /// Why the folder was not flushed after it removed some, so a power
    /// loss could bring them back.
    pub unsynced: Option<io::Error>,
}

/// Deletes the recordings `doomed` in `dir`, for good. A file that cannot
/// be removed (one already gone included) does not stop the others; the
/// first `keep` of them are named, with why. The temporary files saves of
/// a doomed recording left go with it, so no copy of it stays behind under
/// a hidden name; one that cannot be removed fails that recording. Those
/// of any other file stay: a save running in another session keeps its
/// own. The folder is flushed once, after, when anything was removed.
///
/// # Errors
/// [`ChangeError::Missing`] when there is no recordings folder, and
/// [`ChangeError::Failed`] when it cannot be opened or read. Nothing was
/// removed then.
pub fn delete(dir: &Path, doomed: Doomed<'_>, keep: usize) -> Result<Removed, ChangeError> {
    let folder = open(dir)?;
    let unread = |err: io::Error| ChangeError::Failed(folder.named(dir, &err));
    let temps = folder.temps().map_err(unread)?;
    let files = match doomed {
        Doomed::These(files) => files.to_vec(),
        Doomed::All => every(&folder, dir)?,
    };
    let mut removed = Removed::default();
    let mut changed = false;
    for file in files {
        let (gone, outcome) = remove_with_temps(&folder, dir, &file, &temps);
        changed |= gone;
        match outcome {
            Ok(()) => removed.deleted = removed.deleted.saturating_add(1),
            Err(err) if removed.failed.len() < keep => removed.failed.push((file, err)),
            Err(_) => removed.more_failed = removed.more_failed.saturating_add(1),
        }
    }
    if changed {
        removed.unsynced = folder.sync().err();
    }
    Ok(removed)
}

/// Removes `file` and the temporary files `temps` holds for it, all of
/// them even when one fails. Says whether anything was removed, and the
/// first failure: the file's own, or else a temporary file's.
fn remove_with_temps(
    folder: &Folder,
    dir: &Path,
    file: &FileName,
    temps: &[Temp],
) -> (bool, Result<(), ChangeError>) {
    let mut outcome = remove(folder, dir, file);
    let mut gone = outcome.is_ok();
    for temp in temps.iter().filter(|temp| temp.target() == file.as_str()) {
        match folder.remove_file(temp.name()) {
            Ok(()) => gone = true,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                if outcome.is_ok() {
                    let kept =
                        io::Error::new(err.kind(), format!("a hidden copy of it was kept: {err}"));
                    outcome = Err(failed(folder, dir, Step::Deleted, &kept));
                }
            }
        }
    }
    (gone, outcome)
}

/// Every file with a recording's name in the open `folder`. Read whole
/// before any is removed, so removing one never moves what is read next.
fn every(folder: &Folder, dir: &Path) -> Result<Vec<FileName>, ChangeError> {
    let unread = |err: io::Error| ChangeError::Failed(folder.named(dir, &err));
    let mut files = Vec::new();
    for entry in folder.entries().map_err(unread)? {
        let name = entry.map_err(unread)?.file_name();
        if let Some(file) = name.to_str().and_then(FileName::parse) {
            files.push(file);
        }
    }
    Ok(files)
}

/// Removes `file` from the open `folder`.
fn remove(folder: &Folder, dir: &Path, file: &FileName) -> Result<(), ChangeError> {
    match folder.remove_file(file.as_str()) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(ChangeError::Missing),
        Err(err) => Err(failed(folder, dir, Step::Deleted, &err)),
    }
}

/// Opens the recordings folder, refusing a link in its place.
fn open(dir: &Path) -> Result<Folder, ChangeError> {
    Folder::open(dir).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => ChangeError::Missing,
        _ => ChangeError::Failed(err),
    })
}

/// `err`, from a file that could not be through `step`, naming the folder
/// by its path rather than by the handle it was reached through.
fn failed(folder: &Folder, dir: &Path, step: Step, err: &io::Error) -> ChangeError {
    ChangeError::NotDone {
        step,
        err: folder.named(dir, err),
    }
}

#[cfg(test)]
mod tests;
