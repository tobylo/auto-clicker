//! Which recording the playback hotkey plays, kept across restarts in
//! `current-recording` in the program's state folder: one line, the
//! recording's file name. It is state, not a setting, so choosing a
//! recording never rewrites the config file.

use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;

use super::library::FileName;
use crate::dirs::state_dir;
use crate::fs::{
    Link, STALE_TEMP_AGE, atomic_write, create_private_dir_all, open_read, remove_stale_temps,
};

/// How much of the file is read: more than a recording's file name and a
/// newline, so a longer file reads as no file name.
const MAX_BYTES: u64 = 64;

/// Where the current recording is kept; nowhere when there is no state
/// directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CurrentFile(Option<PathBuf>);

impl CurrentFile {
    /// `current-recording` in the program's state folder ([`state_dir`]);
    /// nowhere when there is no such folder.
    pub fn of_environment() -> CurrentFile {
        CurrentFile(state_dir().map(|dir| dir.join("current-recording")))
    }

    /// At `path`.
    pub fn at(path: PathBuf) -> CurrentFile {
        CurrentFile(Some(path))
    }

    /// Nowhere: nothing is read or kept.
    pub fn none() -> CurrentFile {
        CurrentFile(None)
    }

    /// The recording kept; `Ok(None)` when none is, or there is nowhere to
    /// keep one.
    ///
    /// The file is opened without following a link and without waiting
    /// on a FIFO, and must be a regular file holding one recording's file
    /// name.
    ///
    /// # Errors
    /// Why the file cannot be read, or what is wrong with it.
    pub fn read(&self) -> io::Result<Option<FileName>> {
        let Some(path) = &self.0 else {
            return Ok(None);
        };
        let file = match open_read(path, Link::Refuse) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        parse(&read_small(file)?).map(Some)
    }

    /// Keeps `file` as the current recording, atomically, creating the
    /// directory (private to its owner) when it is missing. Does nothing when there is
    /// nowhere to keep it.
    ///
    /// # Errors
    /// The directory cannot be made or the file cannot be written.
    pub fn write(&self, file: &FileName) -> io::Result<()> {
        let Some(path) = &self.0 else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            create_private_dir_all(dir)?;
        }
        atomic_write(path, format!("{file}\n").as_bytes())
    }

    /// Removes the temporary files that writes of this file left when they
    /// crashed at least [`STALE_TEMP_AGE`] ago; how many it removed.
    ///
    /// # Errors
    /// Its directory cannot be read.
    pub fn remove_stale_temps(&self) -> io::Result<usize> {
        match self.0.as_deref() {
            Some(path) => remove_stale_temps(path, STALE_TEMP_AGE),
            None => Ok(0),
        }
    }
}

/// The first [`MAX_BYTES`] of a regular file.
fn read_small(file: File) -> io::Result<Vec<u8>> {
    if !file.metadata()?.is_file() {
        return Err(invalid("it is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES).read_to_end(&mut bytes)?;
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

    use super::*;
    use crate::test_support::TempDir;

    fn name(text: &str) -> FileName {
        FileName::parse(text).unwrap()
    }

    fn in_dir(dir: &TempDir) -> (CurrentFile, PathBuf) {
        let path = dir.path().join("auto-clicker").join("current-recording");
        (CurrentFile::at(path.clone()), path)
    }

    #[test]
    fn the_file_is_in_the_state_folder() {
        assert_eq!(
            CurrentFile::of_environment(),
            state_dir().map_or(CurrentFile::none(), |dir| {
                CurrentFile::at(dir.join("current-recording"))
            })
        );
    }

    #[test]
    fn a_written_recording_reads_back() {
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
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_its_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let (current, path) = in_dir(&dir);

        current
            .write(&name("2026-11-09_18-04-05.123.json"))
            .unwrap();

        let mode =
            |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(path.parent().unwrap()), mode(&path)), (0o700, 0o600));
    }

    #[test]
    fn a_start_removes_only_old_temporary_files() {
        let dir = TempDir::new();
        let (current, path) = in_dir(&dir);
        current
            .write(&name("2026-11-09_18-04-05.123.json"))
            .unwrap();
        let fresh = path.with_file_name(".current-recording.7.0123456789abcdef.tmp");
        fs::write(&fresh, b"half").unwrap();
        let old = path.with_file_name(".current-recording.8.0123456789abcdef.tmp");
        crate::test_support::old_temp(&old);

        let removed = (
            current.remove_stale_temps().unwrap(),
            CurrentFile::none().remove_stale_temps().unwrap(),
        );

        assert_eq!(removed, (1, 0));
        assert!(fresh.exists() && !old.exists());
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
            // Only the start is read: a name followed by more is none.
            (
                b"2026-11-09_18-04-05.123.json\n                                   x",
                false,
            ),
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

    /// `current`'s read, which must end within a few seconds.
    fn read_at_once(current: &CurrentFile) -> io::Result<Option<FileName>> {
        let current = current.clone();
        let (send, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || send.send(current.read()));
        received
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the read waited")
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_or_a_fifo_is_refused_at_once() {
        let dir = TempDir::new();
        let (current, path) = in_dir(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "2026-11-09_18-04-05.123.json\n").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let err = read_at_once(&current).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::ELOOP), "{err}");
        fs::remove_file(&path).unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());
        let not_a_file = |read: io::Result<Option<FileName>>| {
            read.unwrap_err().to_string() == "it is not a regular file"
        };
        assert!(not_a_file(read_at_once(&current)));
    }

    #[test]
    fn a_folder_in_the_files_place_is_refused() {
        let dir = TempDir::new();
        let (current, path) = in_dir(&dir);
        fs::create_dir_all(&path).unwrap();
        let err = read_at_once(&current).unwrap_err();
        assert_eq!(err.to_string(), "it is not a regular file");
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_read_is_an_error() {
        use crate::test_support::permissions_apply;
        use std::os::unix::fs::PermissionsExt;
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
