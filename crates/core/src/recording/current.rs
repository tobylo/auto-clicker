//! Which recording the playback hotkey plays, kept across restarts in
//! `$XDG_STATE_HOME/auto-clicker/current-recording`: one line, the
//! recording's file name. It is state, not a setting, so choosing a
//! recording never rewrites the config file.

use std::ffi::OsString;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;

use super::library::FileName;
use crate::fs::atomic_write;

/// The longest file a valid one can be: a recording's file name and a
/// newline.
const MAX_BYTES: u64 = 64;

/// Where the current recording is kept; nowhere when there is no state
/// directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CurrentFile(Option<PathBuf>);

impl CurrentFile {
    /// `$XDG_STATE_HOME/auto-clicker/current-recording`, or
    /// `~/.local/state/auto-clicker/current-recording` when
    /// `XDG_STATE_HOME` is unset or not absolute.
    pub fn of_environment() -> CurrentFile {
        CurrentFile::from_env(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
    }

    /// At `path`.
    pub fn at(path: PathBuf) -> CurrentFile {
        CurrentFile(Some(path))
    }

    /// Nowhere: nothing is read or kept.
    pub fn none() -> CurrentFile {
        CurrentFile(None)
    }

    fn from_env(xdg_state_home: Option<OsString>, home: Option<OsString>) -> CurrentFile {
        let absolute = |dir: OsString| Some(PathBuf::from(dir)).filter(|dir| dir.is_absolute());
        let dir = xdg_state_home.and_then(absolute).or_else(|| {
            home.and_then(absolute)
                .map(|home| home.join(".local").join("state"))
        });
        CurrentFile(dir.map(|dir| dir.join("auto-clicker").join("current-recording")))
    }

    /// The recording kept; `Ok(None)` when none is, or there is nowhere to
    /// keep one.
    ///
    /// The file is opened without following a symlink and without waiting
    /// on a FIFO, and must be a regular file holding one recording's file
    /// name.
    ///
    /// # Errors
    /// Why the file cannot be read, or what is wrong with it.
    pub fn read(&self) -> io::Result<Option<FileName>> {
        let Some(path) = &self.0 else {
            return Ok(None);
        };
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        parse(&read_small(file)?).map(Some)
    }

    /// Keeps `file` as the current recording, atomically, creating the
    /// directory (mode 0700) when it is missing. Does nothing when there is
    /// nowhere to keep it.
    ///
    /// # Errors
    /// The directory cannot be made or the file cannot be written.
    pub fn write(&self, file: &FileName) -> io::Result<()> {
        let Some(path) = &self.0 else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        atomic_write(path, format!("{file}\n").as_bytes())
    }
}

/// What a regular file holds, up to [`MAX_BYTES`]; more is an error.
fn read_small(file: File) -> io::Result<Vec<u8>> {
    if !file.metadata()?.is_file() {
        return Err(invalid("it is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_BYTES) {
        return Err(invalid("it is too large"));
    }
    Ok(bytes)
}

/// The file name in `bytes`, with or without one trailing newline.
fn parse(bytes: &[u8]) -> io::Result<FileName> {
    let line = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    std::str::from_utf8(line)
        .ok()
        .and_then(FileName::parse)
        .ok_or_else(|| invalid("it does not hold a recording's file name"))
}

fn invalid(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::test_support::{TempDir, permissions_apply};

    fn name(text: &str) -> FileName {
        FileName::parse(text).unwrap()
    }

    fn in_dir(dir: &TempDir) -> (CurrentFile, PathBuf) {
        let path = dir.path().join("auto-clicker").join("current-recording");
        (CurrentFile::at(path.clone()), path)
    }

    #[test]
    fn the_path_follows_xdg_state_home_then_home() {
        let os = |text: &str| Some(OsString::from(text));
        let at = |text: &str| CurrentFile::at(PathBuf::from(text));
        let cases = [
            (
                os("/state"),
                os("/home/u"),
                at("/state/auto-clicker/current-recording"),
            ),
            (
                os("relative"),
                os("/home/u"),
                at("/home/u/.local/state/auto-clicker/current-recording"),
            ),
            (
                None,
                os("/home/u"),
                at("/home/u/.local/state/auto-clicker/current-recording"),
            ),
            (None, os("relative"), CurrentFile::none()),
            (None, None, CurrentFile::none()),
        ];
        for (state, home, expected) in cases {
            assert_eq!(
                CurrentFile::from_env(state.clone(), home.clone()),
                expected,
                "{state:?} {home:?}"
            );
        }
    }

    #[test]
    fn a_written_recording_reads_back_from_a_private_file() {
        let dir = TempDir::new();
        let (current, path) = in_dir(&dir);
        assert_eq!(current.read().unwrap(), None);

        current
            .write(&name("2026-11-09_18-04-05.123.json"))
            .unwrap();
        current
            .write(&name("2026-11-10_07-00-00.000-1.json"))
            .unwrap();

        assert_eq!(
            current.read().unwrap(),
            Some(name("2026-11-10_07-00-00.000-1.json"))
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            b"2026-11-10_07-00-00.000-1.json\n"
        );
        let mode =
            |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(path.parent().unwrap()), mode(&path)), (0o700, 0o600));
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn a_file_without_one_file_name_is_an_error_not_nothing() {
        let dir = TempDir::new();
        let (current, path) = in_dir(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let valid = "2026-11-09_18-04-05.123.json";
        let cases: [(&[u8], bool); 8] = [
            (valid.as_bytes(), true),
            (b"2026-11-09_18-04-05.123.json\n", true),
            (b"2026-11-09_18-04-05.123.json\n\n", false),
            (b" 2026-11-09_18-04-05.123.json", false),
            (b"../2026-11-09_18-04-05.123.json", false),
            (b"", false),
            (b"\xff\xfe", false),
            (&[b'x'; 65], false),
        ];
        for (bytes, reads) in cases {
            fs::write(&path, bytes).unwrap();
            let read = current.read();
            assert_eq!(read.is_ok(), reads, "{bytes:?}: {read:?}");
            if let Err(err) = read {
                assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{bytes:?}");
            }
        }
    }

    #[test]
    fn a_symlink_a_fifo_or_a_folder_is_refused_at_once() {
        let dir = TempDir::new();
        let (current, path) = in_dir(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "2026-11-09_18-04-05.123.json\n").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(current.read().is_err());
        fs::remove_file(&path).unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());
        assert!(current.read().is_err());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(current.read().is_err());
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_error() {
        let dir = TempDir::new();
        let (current, path) = in_dir(&dir);
        current
            .write(&name("2026-11-09_18-04-05.123.json"))
            .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if !permissions_apply() {
            return;
        }
        let err = current.read().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn nowhere_reads_none_and_keeps_nothing() {
        let current = CurrentFile::none();
        current
            .write(&name("2026-11-09_18-04-05.123.json"))
            .unwrap();
        assert_eq!(current.read().unwrap(), None);
    }
}
