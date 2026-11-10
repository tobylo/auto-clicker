//! Writing files so that a crash never leaves half of one.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use nix::fcntl::{RenameFlags, renameat2};

/// Replaces the file at `path` with `contents`, atomically.
///
/// Writes a temporary file in the same directory, flushes it to disk, renames
/// it over `path` and flushes the directory, so after a crash `path` holds
/// either its old contents or the new ones, never a mix. If a step before the
/// rename fails, the temporary file is removed and `path` is untouched. If
/// only flushing the directory fails, the error says that the new contents
/// are in place but may not survive a power loss.
///
/// The new file is readable and writable by its owner only (mode 0600):
/// recordings hold keystrokes.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = parent_dir(path)?;
    write_then_place(
        dir,
        path,
        |out| out.write_all(contents),
        |temp| fs::rename(temp, path).map(|()| path.to_path_buf()),
    )
    .map(drop)
}

/// Writes a new file, atomically, never replacing one: at the first of
/// `paths` that does not exist yet. All of `paths` must be in one directory.
/// Returns the path written.
///
/// Like [`atomic_write`], the contents go to a temporary file that is
/// flushed to disk before it is renamed into place, and the directory is
/// flushed after, so a crash leaves no half-written file. The rename uses
/// `RENAME_NOREPLACE`: a file that appears at a path between the check and
/// the rename is never overwritten. The new file has mode 0600.
///
/// # Errors
/// `write`'s error, a write or flush error, or `AlreadyExists` when every
/// path is taken. The temporary file is removed on any error before the
/// rename.
pub fn atomic_create(
    paths: &[PathBuf],
    write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let first = paths.first().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "no path to write the file at")
    })?;
    let dir = parent_dir(first)?;
    if paths.iter().any(|path| parent_dir(path).ok() != Some(dir)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the paths are not in one directory",
        ));
    }
    write_then_place(dir, first, write, |temp| place_new(temp, paths))
}

/// Renames `temp` to the first of `paths` that is free.
fn place_new(temp: &Path, paths: &[PathBuf]) -> io::Result<PathBuf> {
    for path in paths {
        match renameat2(
            None,
            temp,
            None,
            path.as_path(),
            RenameFlags::RENAME_NOREPLACE,
        ) {
            Ok(()) => return Ok(path.clone()),
            Err(nix::errno::Errno::EEXIST) => {}
            Err(errno) => return Err(errno.into()),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "{} and every other name tried are taken",
            display_first(paths)
        ),
    ))
}

fn display_first(paths: &[PathBuf]) -> String {
    paths
        .first()
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

/// The steps both writers share: write a temporary file beside `path` with
/// `write`, flush it to disk, `place` it, and flush `dir`. A failure before
/// the file is placed removes the temporary file.
fn write_then_place(
    dir: &Path,
    path: &Path,
    write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    place: impl FnOnce(&Path) -> io::Result<PathBuf>,
) -> io::Result<PathBuf> {
    let (temp, file) = create_temp(dir, path)?;
    let placed = fill(file, write).and_then(|()| place(&temp));
    let placed = match placed {
        Ok(placed) => placed,
        Err(err) => return Err(remove_temp(&temp, err)),
    };
    File::open(dir)
        .and_then(|dir| dir.sync_all())
        .map_err(|err| {
            io::Error::new(
                err.kind(),
                format!(
                    "{} was written, but syncing its directory failed, so the change may not \
                 survive a power loss: {err}",
                    placed.display()
                ),
            )
        })?;
    Ok(placed)
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

/// A new file beside `path`, named so it is hidden and cannot clash with
/// another writer's: `create_new` fails rather than reuse an existing file.
fn create_temp(dir: &Path, path: &Path) -> io::Result<(PathBuf, File)> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} names no file", path.display()),
        )
    })?;
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(
        ".{}.{:016x}.tmp",
        std::process::id(),
        fastrand::u64(..)
    ));
    let temp = dir.join(temp_name);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    Ok((temp, file))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::test_support::{TempDir, permissions_apply};

    #[test]
    fn writes_a_new_file_and_leaves_nothing_else() {
        let dir = TempDir::new();
        let path = dir.path().join("config.toml");

        atomic_write(&path, b"version = 1\n").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"version = 1\n");
        assert_eq!(dir.entries(), ["config.toml"]);
    }

    #[test]
    fn the_new_file_is_private_to_its_owner() {
        let dir = TempDir::new();
        let path = dir.path().join("recording.json");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        atomic_write(&path, b"new").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn replaces_an_existing_file_whole() {
        let dir = TempDir::new();
        let path = dir.path().join("config.toml");
        fs::write(&path, b"a much longer old content than the new one").unwrap();

        atomic_write(&path, b"new").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(dir.entries(), ["config.toml"]);
    }

    #[test]
    fn a_failed_rename_removes_the_temp_file_and_leaves_the_target() {
        let dir = TempDir::new();
        // A non-empty directory cannot be replaced by a file.
        let path = dir.path().join("taken");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("inside"), b"kept").unwrap();

        assert!(atomic_write(&path, b"new").is_err());

        assert_eq!(dir.entries(), ["taken"]);
        assert_eq!(fs::read(path.join("inside")).unwrap(), b"kept");
    }

    #[test]
    fn a_failed_directory_sync_says_the_file_was_replaced() {
        if !permissions_apply() {
            return;
        }
        let dir = TempDir::new();
        // Writable and searchable but not readable: the rename works, and
        // opening the directory to sync it fails.
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o300)).unwrap();
        let path = locked.join("config.toml");

        let result = atomic_write(&path, b"new");

        // Restored before any assertion, so the directory can be removed.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        let message = result.err().map(|err| err.to_string()).unwrap_or_default();
        let replaced = message.contains("was written, but syncing its directory");
        let contents = fs::read(&path).unwrap();
        assert_eq!((replaced, contents), (true, b"new".to_vec()), "{message}");
    }

    #[test]
    fn a_missing_directory_is_an_error() {
        let dir = TempDir::new();
        let path = dir.path().join("missing").join("config.toml");

        let err = atomic_write(&path, b"x").unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(dir.entries(), Vec::<String>::new());
    }

    #[test]
    fn a_bare_file_name_is_in_the_current_directory() {
        let dirs = (
            parent_dir(Path::new("config.toml")).unwrap(),
            parent_dir(Path::new("/etc/config.toml")).unwrap(),
        );
        assert_eq!(dirs, (Path::new("."), Path::new("/etc")));
    }

    #[test]
    fn a_path_without_a_file_name_is_an_error() {
        let dir = TempDir::new();
        for path in [Path::new("/").to_owned(), dir.path().join("..")] {
            let err = atomic_write(&path, b"x").unwrap_err();
            assert_eq!(
                err.kind(),
                io::ErrorKind::InvalidInput,
                "{}",
                path.display()
            );
        }
    }
    #[test]
    fn create_writes_the_first_free_path_and_never_replaces() {
        let dir = TempDir::new();
        let paths = ["a.json", "a-1.json", "a-2.json"].map(|name| dir.path().join(name));
        fs::write(&paths[0], b"kept").unwrap();

        let first = atomic_create(&paths, |out| out.write_all(b"one")).unwrap();
        let second = atomic_create(&paths, |out| out.write_all(b"two")).unwrap();
        let full = atomic_create(&paths, |out| out.write_all(b"three")).unwrap_err();

        assert_eq!((first, second), (paths[1].clone(), paths[2].clone()));
        assert_eq!(full.kind(), io::ErrorKind::AlreadyExists);
        let named = paths[0].display().to_string();
        assert!(full.to_string().starts_with(&named), "{full}");
        let contents = paths.clone().map(|path| fs::read(path).unwrap());
        assert_eq!(
            contents,
            [b"kept".to_vec(), b"one".to_vec(), b"two".to_vec()]
        );
        assert_eq!(dir.entries(), ["a-1.json", "a-2.json", "a.json"]);
        let mode = fs::metadata(&paths[1]).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_failed_write_leaves_no_file_behind() {
        let dir = TempDir::new();
        let path = dir.path().join("recording.json");

        let err = atomic_create(std::slice::from_ref(&path), |out| {
            out.write_all(b"half a reco")?;
            Err(io::Error::other("crashed mid-write"))
        })
        .unwrap_err();

        assert_eq!(err.to_string(), "crashed mid-write");
        assert_eq!(dir.entries(), Vec::<String>::new());
    }

    #[test]
    fn create_refuses_paths_in_several_directories_or_none() {
        let dir = TempDir::new();
        let paths = [dir.path().join("a"), dir.path().join("sub").join("b")];
        let kinds = [
            atomic_create(&paths, |_| Ok(())).unwrap_err().kind(),
            atomic_create(&[], |_| Ok(())).unwrap_err().kind(),
        ];
        assert_eq!(kinds, [io::ErrorKind::InvalidInput; 2]);
        assert_eq!(dir.entries(), Vec::<String>::new());
    }
}
