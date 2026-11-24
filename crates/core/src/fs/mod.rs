//! Writing files so that a crash never leaves half of one, and working in a
//! checked folder so that a folder moved or replaced meanwhile is never
//! written to.
//!
//! The rules live here, once, for both systems; `unix.rs` and `windows.rs`
//! hold the few calls that differ. A save writes a temporary file beside its
//! target, flushes it to disk and puts it in place: on Linux by a rename
//! (`RENAME_NOREPLACE` where nothing may be replaced), on Windows by a
//! rename or, where nothing may be replaced, a hard link and then removing
//! the temporary file. Linux then flushes the directory; Windows has no
//! such call, so that step is left out there.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

mod patience;
mod temps;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as sys;
#[cfg(windows)]
use windows as sys;

use patience::{Op, Patience};
pub use temps::{Temp, remove_stale_temps};

/// How old a temporary file must be before a start removes it: far longer
/// than any save takes, so a save still running in another session keeps
/// its file.
pub const STALE_TEMP_AGE: Duration = Duration::from_secs(60 * 60);

/// Replaces the file at `path` with `contents`, atomically.
///
/// Writes a temporary file in the same directory, flushes it to disk, renames
/// it over `path` and flushes the directory, so after a crash `path` holds
/// either its old contents or the new ones, never a mix. If a step before the
/// rename fails, the temporary file is removed and `path` is untouched. If
/// only flushing the directory fails, the error says that the new contents
/// are in place but may not survive a power loss.
///
/// On Linux the new file is readable and writable by its owner only (mode
/// 0600): recordings hold keystrokes. On Windows it takes the permissions of
/// its folder, which under the user's profile only the user can read.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    atomic_save(path, contents, Place::Replace)?.map_err(Unsynced::into_error)
}

/// How a save puts the new file in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Over whatever is at the path.
    Replace,
    /// Only where nothing is: `AlreadyExists` if a file appeared.
    New,
}

/// The new contents are in place, but flushing their directory failed, so
/// the change may not survive a power loss.
#[derive(Debug)]
pub struct Unsynced {
    placed: PathBuf,
    error: io::Error,
}

impl Unsynced {
    pub fn into_error(self) -> io::Error {
        io::Error::new(
            self.error.kind(),
            format!(
                "{} was written, but syncing its directory failed, so the change may not \
                 survive a power loss: {}",
                self.placed.display(),
                self.error
            ),
        )
    }
}

/// [`atomic_write`], placing the file as `place` says, and telling a write
/// that failed (the outer error: `path` is untouched) from one that is in
/// place but not flushed to disk (the inner one).
pub fn atomic_save(path: &Path, contents: &[u8], place: Place) -> io::Result<Result<(), Unsynced>> {
    let dir = parent_dir(path)?;
    let patience = Patience::new();
    let placed = write_then_place(
        Beside { dir, path },
        |out| out.write_all(contents),
        |temp| put(temp, path, place, &patience).map(|()| path.to_path_buf()),
        || sys::sync_dir(dir),
    )?;
    Ok(placed.map(drop))
}

/// Opens the file at `path` for reading, without waiting on a FIFO, and,
/// where `link` says so, refusing a symbolic link (or, on Windows, any
/// reparse point) in its place. A file another program holds open for a
/// moment (a virus scanner, the search indexer) is tried again for a
/// moment, and then reported as busy, never as missing.
pub fn open_read(path: &Path, link: Link) -> io::Result<File> {
    Patience::new().retry(path, Op::Read, || sys::open_read(path, link))
}

/// Whether [`open_read`] follows a link at the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Follow,
    Refuse,
}

/// Creates `dir` and every missing directory above it, each readable by
/// its owner only on Linux (mode 0700); on Windows they take the
/// permissions of the folder they are made in.
pub fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    sys::create_private_dir(dir)
}

/// Flushes the directory `dir` to disk, so the entries in it survive a
/// power loss. Windows has no such call: there it does nothing.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    sys::sync_dir(dir)
}

/// A folder, opened and checked once, that every later step works in.
///
/// It is refused when it is a symbolic link (on Windows, any reparse
/// point, a junction included). On Linux each name is reached through the
/// open folder itself, so a folder moved meanwhile still gets the file and
/// whatever took its place does not. On Windows its path is resolved once
/// into one that goes through no link, and the folder is held open so that
/// nobody can rename or delete it while it is held: that path keeps naming
/// the folder that was checked.
///
/// Every file call in it shares one wait for a file another program holds
/// for a moment: about 300 ms at most for the whole operation.
pub struct Folder {
    handle: File,
    /// What each name is joined to.
    path: PathBuf,
    patience: Patience,
}

impl Folder {
    /// Opens `dir`, refusing a link in its place.
    pub fn open(dir: &Path) -> io::Result<Folder> {
        let (handle, path) = sys::open_dir(dir)?;
        Ok(Folder {
            handle,
            path,
            patience: Patience::new(),
        })
    }

    /// Makes `dir` (absolute) exist and be readable by its owner only, and
    /// opens it. On Linux an existing one that others may read is closed to
    /// them, and its ancestors are flushed every time, up to the first this
    /// user may not read, so the directory survives a power loss whichever
    /// save created which part of the path. A directory the user may not
    /// list (`/home` at mode 0711, say) is none their saves made.
    pub fn open_private(dir: &Path) -> io::Result<Folder> {
        if !dir.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not an absolute path", dir.display()),
            ));
        }
        sys::create_private_dir(dir)?;
        let folder = Folder::open(dir)?;
        sys::make_private(&folder.handle)?;
        for ancestor in dir.ancestors().skip(1) {
            match sys::sync_dir(ancestor) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::PermissionDenied => break,
                Err(err) => return Err(err),
            }
        }
        Ok(folder)
    }

    /// The path of the file `name` in the folder. A name is one plain path
    /// component: anything that could reach outside the folder is refused.
    fn join(&self, name: &str) -> io::Result<PathBuf> {
        let mut parts = Path::new(name).components();
        match (parts.next(), parts.next()) {
            (Some(Component::Normal(part)), None) if part == name => Ok(self.path.join(name)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name:?} is not a file name"),
            )),
        }
    }

    /// The path names in the folder are joined to: on Linux one that goes
    /// through the open folder, on Windows the folder's resolved path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Opens the file `name` in the folder for reading, refusing a link and
    /// without waiting on a FIFO.
    pub fn open_file(&self, name: &str) -> io::Result<File> {
        let path = self.join(name)?;
        self.patience
            .retry(&path, Op::Read, || sys::open_read(&path, Link::Refuse))
    }

    /// The folder's entries.
    pub fn entries(&self) -> io::Result<fs::ReadDir> {
        fs::read_dir(&self.path)
    }

    /// Removes the file `name` from the folder.
    pub fn remove_file(&self, name: &str) -> io::Result<()> {
        let path = self.join(name)?;
        self.patience
            .retry(&path, Op::Change, || fs::remove_file(&path))
    }

    /// Flushes the folder to disk; nothing on Windows.
    pub fn sync(&self) -> io::Result<()> {
        sys::sync_open_dir(&self.handle)
    }

    /// Writes a new file, atomically, never replacing one: at the first of
    /// `names` that is free. Returns the name written.
    ///
    /// The contents go to a temporary file that is flushed to disk before
    /// it is put in place, and the folder is flushed after, so a crash
    /// leaves no half-written file. A file that appears under a name
    /// between the check and the placing is never overwritten. Before each
    /// name after the first is tried, the temporary file is renamed after
    /// it, so one left behind always names the file it became.
    ///
    /// # Errors
    /// `write`'s error, a write or flush error, or `AlreadyExists` when
    /// every name is taken. The temporary file is removed on any error
    /// before the file is in place.
    pub fn create_new(
        &self,
        names: &[String],
        write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    ) -> io::Result<String> {
        let first = names.first().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "no name to write the file at")
        })?;
        let path = self.join(first)?;
        let placed = write_then_place(
            Beside {
                dir: &self.path,
                path: &path,
            },
            write,
            |temp| self.place_at_first_free(temp, names),
            || self.sync(),
        )?;
        let placed = placed.map_err(Unsynced::into_error)?;
        placed
            .file_name()
            .and_then(|name| name.to_str())
            .map(ToOwned::to_owned)
            .ok_or_else(|| io::Error::other("the saved file has no name"))
    }

    /// Puts `temp` in place under the first of `names` that is free,
    /// renaming it after each name before that name is tried.
    fn place_at_first_free(&self, temp: &mut PathBuf, names: &[String]) -> io::Result<PathBuf> {
        for (tried, name) in names.iter().enumerate() {
            if let Some(placed) = self.place_if_free(temp, tried > 0, name)? {
                return Ok(placed);
            }
        }
        let first = names.first().map(String::as_str).unwrap_or_default();
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "{} and every other name tried are taken",
                self.path.join(first).display()
            ),
        ))
    }

    /// Puts `temp` at `name` if nothing is there, renaming it after `name`
    /// first when it was named after another (`retarget`). `None` when
    /// `name` is taken.
    fn place_if_free(
        &self,
        temp: &mut PathBuf,
        retarget: bool,
        name: &str,
    ) -> io::Result<Option<PathBuf>> {
        let path = self.join(name)?;
        if retarget {
            self.retarget(temp, name)?;
        }
        match put(temp, &path, Place::New, &self.patience) {
            Ok(()) => Ok(Some(path)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Renames the temporary file `temp` after `name`. Its new name is as
    /// unique as any temporary file's, so the rename replaces nothing.
    fn retarget(&self, temp: &mut PathBuf, name: &str) -> io::Result<()> {
        let next = self.path.join(temps::temp_name(name.as_ref()));
        self.patience
            .retry(&next, Op::Change, || fs::rename(&*temp, &next))?;
        *temp = next;
        Ok(())
    }

    /// Replaces the file `name` with what `write` writes, atomically, as
    /// [`atomic_save`] does with [`Place::Replace`], streamed to the
    /// temporary file rather than held whole in memory.
    pub fn replace(
        &self,
        name: &str,
        write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    ) -> io::Result<Result<(), Unsynced>> {
        let path = self.join(name)?;
        let placed = write_then_place(
            Beside {
                dir: &self.path,
                path: &path,
            },
            write,
            |temp| put(temp, &path, Place::Replace, &self.patience).map(|()| path.clone()),
            || self.sync(),
        )?;
        Ok(placed.map(drop))
    }

    /// The temporary files saves left in the folder: what a crash, or on
    /// Windows a temporary file another program held, left behind, and
    /// those of saves still running.
    ///
    /// # Errors
    /// The folder, or an entry in it, cannot be read.
    pub fn temps(&self) -> io::Result<Vec<Temp>> {
        temps::list(&self.path)
    }

    /// Removes the temporary files saves left in the folder that are at
    /// least `age` old. Returns how many it removed.
    ///
    /// # Errors
    /// The folder cannot be read, or a file could not be removed (the first
    /// such error; the others are still tried).
    pub fn remove_temps(&self, age: Duration) -> io::Result<usize> {
        temps::remove_stale_in(&self.path, age, |path| {
            self.patience
                .retry(path, Op::Change, || fs::remove_file(path))
        })
    }

    /// `err`, naming the folder by `dir`, or where it is now, rather than
    /// by the path its names were reached through.
    pub fn named(&self, dir: &Path, err: &io::Error) -> io::Error {
        let now_at = sys::now_at(&self.path, dir);
        let message = err.to_string().replace(
            &self.path.display().to_string(),
            &now_at.display().to_string(),
        );
        io::Error::new(err.kind(), message)
    }
}

/// Puts the flushed temporary file `temp` at `path`, as `place` says.
fn put(temp: &Path, path: &Path, place: Place, patience: &Patience) -> io::Result<()> {
    match place {
        Place::Replace => patience.retry(path, Op::Change, || fs::rename(temp, path)),
        Place::New => sys::place_new(temp, path, patience),
    }
}

/// Where a temporary file goes: in `dir`, named after `path`, its target.
#[derive(Clone, Copy)]
struct Beside<'a> {
    dir: &'a Path,
    path: &'a Path,
}

/// The steps every save shares: write a temporary file beside the target
/// with `write`, flush it to disk, `place` it (which may rename it first),
/// then `sync` its directory. A failure before the file is placed removes
/// the temporary file, wherever `place` left it, and is the outer error; a
/// failure to sync after is the inner one.
fn write_then_place(
    beside: Beside<'_>,
    write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    place: impl FnOnce(&mut PathBuf) -> io::Result<PathBuf>,
    sync: impl FnOnce() -> io::Result<()>,
) -> io::Result<Result<PathBuf, Unsynced>> {
    let (mut temp, file) = create_temp(beside)?;
    let placed = match fill(file, write).and_then(|()| place(&mut temp)) {
        Ok(placed) => placed,
        Err(err) => return Err(remove_temp(&temp, err)),
    };
    Ok(match sync() {
        Ok(()) => Ok(placed),
        Err(error) => Err(Unsynced { placed, error }),
    })
}

/// Writes through a buffer and flushes the file to disk; the file is closed
/// when this returns.
fn fill(file: File, write: impl FnOnce(&mut dyn Write) -> io::Result<()>) -> io::Result<()> {
    let mut out = BufWriter::new(file);
    write(&mut out)?;
    let file = out.into_inner().map_err(io::IntoInnerError::into_error)?;
    file.sync_all()
}

/// Removes the temporary file after `err`; the write's error stays the one
/// reported, with the file it left if removing fails too.
fn remove_temp(temp: &Path, err: io::Error) -> io::Error {
    match fs::remove_file(temp) {
        Ok(()) => err,
        Err(cleanup) => io::Error::new(
            err.kind(),
            format!("{err}; removing {} failed too: {cleanup}", temp.display()),
        ),
    }
}

/// The directory `path` is in; a bare file name is in the current directory.
fn parent_dir(path: &Path) -> io::Result<&Path> {
    match path.parent() {
        Some(dir) if dir.as_os_str().is_empty() => Ok(Path::new(".")),
        Some(dir) => Ok(dir),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} names no file", path.display()),
        )),
    }
}

/// A new file beside the target, named so it is hidden, is never listed as
/// a recording, and cannot clash with another writer's: creating it fails
/// rather than reuse an existing file.
fn create_temp(beside: Beside<'_>) -> io::Result<(PathBuf, File)> {
    let name = beside.path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} names no file", beside.path.display()),
        )
    })?;
    let temp = beside.dir.join(temps::temp_name(name));
    let file = sys::create_temp(&temp)?;
    Ok((temp, file))
}

#[cfg(test)]
mod tests;
