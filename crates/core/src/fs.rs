//! Writing files so that a crash never leaves half of one.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Replaces the file at `path` with `contents`, atomically.
///
/// Writes a temporary file in the same directory, flushes it to disk, renames
/// it over `path` and flushes the directory, so after a crash `path` holds
/// either its old contents or the new ones, never a mix. If a step before the
/// rename fails, the temporary file is removed and `path` is untouched. If
/// flushing the directory fails, the new contents are in place but may not
/// survive a power loss, and the error says so.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = parent_dir(path)?;
    let (temp, mut file) = create_temp(dir, path)?;
    let written = file
        .write_all(contents)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temp, path));
    drop(file);
    if let Err(err) = written {
        // The temp file is ours and nothing else names it; if removing it
        // fails too, the write's error is the one worth reporting.
        let _ = fs::remove_file(&temp);
        return Err(err);
    }
    File::open(dir).and_then(|dir| dir.sync_all())
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
        .open(&temp)?;
    Ok((temp, file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn writes_a_new_file_and_leaves_nothing_else() {
        let dir = TempDir::new();
        let path = dir.path().join("config.toml");

        atomic_write(&path, b"version = 1\n").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"version = 1\n");
        assert_eq!(dir.entries(), ["config.toml"]);
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
        let err = atomic_write(Path::new("/"), b"x").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
