//! The file calls that differ on Windows, through std alone: core makes no
//! Win32 call of its own (brief decision 3).
//!
//! The constants are the Win32 values of the names they carry.

use std::fs::{self, File, Metadata, OpenOptions};
use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use super::Link;
use super::patience::{Op, Patience};

const FILE_SHARE_READ: u32 = 0x1;
const FILE_SHARE_WRITE: u32 = 0x2;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_LOCK_VIOLATION: i32 = 33;

/// A sharing or lock violation is another program holding the file for a
/// moment. A change may also be refused as "access denied" while one does,
/// or while a removed file is still open somewhere.
pub(super) fn transient(err: &io::Error, op: Op) -> bool {
    match err.raw_os_error() {
        Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION) => true,
        Some(ERROR_ACCESS_DENIED) => op == Op::Change,
        _ => false,
    }
}

/// A new file at `path`; it takes its folder's permissions. std opens a
/// new file without following a reparse point at its path.
pub(super) fn create_temp(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Links `path` to the flushed file `temp` only if nothing is there
/// (`AlreadyExists` otherwise), then removes `temp`. If another program
/// holds `temp` past the deadline, the file is still in place, and the
/// next start removes the temporary name.
pub(super) fn place_new(temp: &Path, path: &Path, patience: &Patience) -> io::Result<()> {
    patience.retry(path, Op::Change, || fs::hard_link(temp, path))?;
    let _left_for_the_next_start = patience.retry(temp, Op::Change, || fs::remove_file(temp));
    Ok(())
}

/// Windows has no directory flush: an entry is on disk once the file
/// system's own journal has it (brief decision 12).
pub(super) fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

pub(super) fn sync_open_dir(_dir: &File) -> io::Result<()> {
    Ok(())
}

/// Opens `path` for reading, letting other programs read, write and remove
/// it meanwhile, and, where `link` refuses one, refusing a reparse point
/// (a symbolic link or a junction) without following it. Windows has no
/// FIFOs at a file path, so nothing here can block.
pub(super) fn open_read(path: &Path, link: Link) -> io::Result<File> {
    // Backup semantics opens a folder too, as Linux does, so a folder or a
    // junction in a file's place reaches the checks rather than failing as
    // "access denied"; callers read only a regular file.
    let no_follow = match link {
        Link::Follow => 0,
        Link::Refuse => FILE_FLAG_OPEN_REPARSE_POINT,
    };
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | no_follow);
    let file = options.open(path)?;
    if link == Link::Refuse && is_reparse_point(&file.metadata()?) {
        return Err(io::Error::other(format!(
            "{} is a link, which is not followed",
            path.display()
        )));
    }
    Ok(file)
}

/// Opens the folder `dir` and holds it, refusing it when it is a reparse
/// point or not a folder.
///
/// Its path is resolved once into one through no link, the folder is opened
/// at that path without letting anyone remove or rename it while it is
/// held, and the path is checked to resolve to itself still, so no link was
/// put on the way between the two. Names in the folder are joined to that
/// path from then on.
pub(super) fn open_dir(dir: &Path) -> io::Result<(File, PathBuf)> {
    refuse_link(&fs::symlink_metadata(dir)?, dir)?;
    let resolved = fs::canonicalize(dir)?;
    let handle = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&resolved)?;
    refuse_link(&handle.metadata()?, dir)?;
    if fs::canonicalize(&resolved)? != resolved {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} changed while it was being opened", dir.display()),
        ));
    }
    Ok((handle, resolved))
}

/// `NotADirectory` unless `metadata` is a folder's that is no reparse point.
fn refuse_link(metadata: &Metadata, dir: &Path) -> io::Result<()> {
    if metadata.is_dir() && !is_reparse_point(metadata) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::NotADirectory,
        format!("{} is a link or not a folder", dir.display()),
    ))
}

fn is_reparse_point(metadata: &Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// Creates `dir` and its missing ancestors; they take the permissions of
/// the folder they are made in, which under the user's profile only the
/// user (and the system and administrators) can read.
pub(super) fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

/// The folder keeps the permissions it inherits from the user's profile:
/// changing them would take Win32 calls core does not make.
pub(super) fn make_private(_dir: &File) -> io::Result<()> {
    Ok(())
}

/// The folder is held where it was opened: it is where `dir` names it.
pub(super) fn now_at(_held: &Path, dir: &Path) -> PathBuf {
    dir.to_path_buf()
}

#[cfg(test)]
mod tests;
