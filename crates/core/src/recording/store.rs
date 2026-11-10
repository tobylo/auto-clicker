//! Where recordings are kept: `$XDG_DATA_HOME/auto-clicker/recordings/`, one
//! file per recording, named after the time it was made.

use std::ffi::OsString;
use std::fs::{DirBuilder, File, OpenOptions, Permissions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::Recording;
use crate::fs::atomic_create;

/// How many names a save tries: the plain one, then `-1` to `-9`. Two
/// recordings made in the same millisecond, or in the hour the clocks go
/// back, would otherwise clash.
const NAMES_TRIED: u32 = 10;

/// The recordings directory: `$XDG_DATA_HOME/auto-clicker/recordings`, or
/// `~/.local/share/auto-clicker/recordings` when `XDG_DATA_HOME` is unset or
/// not absolute. `None` when neither gives an absolute directory.
pub fn recordings_dir() -> Option<PathBuf> {
    recordings_dir_from(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

fn recordings_dir_from(xdg_data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let absolute = |dir: OsString| Some(PathBuf::from(dir)).filter(|dir| dir.is_absolute());
    xdg_data_home
        .and_then(absolute)
        .or_else(|| {
            home.and_then(absolute)
                .map(|home| home.join(".local").join("share"))
        })
        .map(|dir| dir.join("auto-clicker").join("recordings"))
}

/// Saves `recording` in `dir`, creating the directory (mode 0700) if
/// needed, as a new file (mode 0600) named after its creation time. Never
/// replaces a file. Returns the file's path, as `dir` named it.
///
/// The directory is opened and checked once, and the file is written,
/// renamed into place and flushed through that open directory, so a
/// directory renamed or replaced meanwhile never receives it.
///
/// # Errors
/// `dir` is relative, is a symlink, cannot be created or made private;
/// the file cannot be written; or every name tried is taken.
pub fn save(dir: &Path, recording: &Recording) -> io::Result<PathBuf> {
    let checked = private_dir(dir)?;
    let name = save_in(&checked, recording)?;
    Ok(dir.join(name))
}

/// Saves `recording` in the open directory `checked`; the file's name.
fn save_in(checked: &File, recording: &Recording) -> io::Result<OsString> {
    // The kernel resolves this through the open directory, not its name.
    let held = PathBuf::from(format!("/proc/self/fd/{}", checked.as_raw_fd()));
    let stem = recording.created().file_stem();
    let paths: Vec<PathBuf> = (0..NAMES_TRIED)
        .map(|n| match n {
            0 => held.join(format!("{stem}.json")),
            n => held.join(format!("{stem}-{n}.json")),
        })
        .collect();
    let written = atomic_create(&paths, |out| recording.write_json(out))?;
    written
        .file_name()
        .map(ToOwned::to_owned)
        .ok_or_else(|| io::Error::other("the saved file has no name"))
}

/// Makes `dir` (absolute) exist and be readable by its owner only, and
/// returns it open. An
/// existing one that others may read is closed to them; one this user does
/// not own, or a symlink in its place, fails. Its ancestors are flushed every
/// time, up to the first this user may not read, so the directory survives a
/// power loss whichever save, failed or not, created which part of the path.
/// A directory the user may not list (`/home` at mode 0711, say) is none
/// their saves made; flushing one with nothing new in it costs next to
/// nothing.
fn private_dir(dir: &Path) -> io::Result<File> {
    if !dir.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not an absolute path", dir.display()),
        ));
    }
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let handle = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)?;
    if handle.metadata()?.permissions().mode() & 0o077 != 0 {
        handle.set_permissions(Permissions::from_mode(0o700))?;
    }
    for ancestor in dir.ancestors().skip(1) {
        match open_dir(ancestor) {
            Ok(opened) => opened.sync_all()?,
            Err(err) if err.kind() == io::ErrorKind::PermissionDenied => break,
            Err(err) => return Err(err),
        }
    }
    Ok(handle)
}

/// Opens a directory to flush it; anything else in its place (a FIFO,
/// which would block an ordinary open) fails at once.
fn open_dir(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::recording::tests::sample;
    use crate::test_support::TempDir;

    #[test]
    fn the_directory_follows_xdg_then_home() {
        let os = |text: &str| Some(OsString::from(text));
        let cases = [
            (
                os("/data"),
                os("/home/u"),
                Some("/data/auto-clicker/recordings"),
            ),
            (
                None,
                os("/home/u"),
                Some("/home/u/.local/share/auto-clicker/recordings"),
            ),
            (
                os("relative"),
                os("/home/u"),
                Some("/home/u/.local/share/auto-clicker/recordings"),
            ),
            (None, os("relative"), None),
            (None, None, None),
        ];
        for (xdg, home, expected) in cases {
            let found = recordings_dir_from(xdg.clone(), home.clone());
            assert_eq!(found, expected.map(PathBuf::from), "{xdg:?} {home:?}");
        }
    }

    #[test]
    fn saves_a_private_file_in_a_private_directory_and_reads_back() {
        let root = TempDir::new();
        let dir = root.path().join("auto-clicker").join("recordings");
        let recording = sample();

        let path = save(&dir, &recording).unwrap();

        assert_eq!(path, dir.join("2026-11-09_18-04-05.123.json"));
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&dir), mode(&path)), (0o700, 0o600));
        let read = Recording::from_json(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(read, recording);
    }

    #[test]
    fn an_existing_directory_others_can_read_is_made_private() {
        let root = TempDir::new();
        let dir = root.path().join("recordings");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        save(&dir, &sample()).unwrap();

        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn a_new_nested_data_directory_is_created_whole() {
        let root = TempDir::new();
        let dir = root.path().join("new/data/auto-clicker/recordings");
        save(&dir, &sample()).unwrap();
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn an_ancestor_that_may_be_searched_but_not_read_ends_the_flush() {
        if !crate::test_support::permissions_apply() {
            return;
        }
        let root = TempDir::new();
        let closed = root.path().join("closed");
        let dir = closed.join("alice/data/auto-clicker/recordings");
        fs::create_dir_all(closed.join("alice")).unwrap();
        fs::set_permissions(&closed, fs::Permissions::from_mode(0o311)).unwrap();

        let saved = save(&dir, &sample());

        fs::set_permissions(&closed, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(saved.is_ok(), "{saved:?}");
    }

    #[test]
    fn a_fifo_in_place_of_a_directory_fails_at_once() {
        let root = TempDir::new();
        let fifo = root.path().join("fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let err = open_dir(&fifo).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::ENOTDIR), "{err}");
    }

    #[test]
    fn a_symlink_or_a_relative_path_is_refused_and_nothing_changes() {
        let root = TempDir::new();
        let elsewhere = root.path().join("project");
        fs::create_dir(&elsewhere).unwrap();
        fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o755)).unwrap();
        let link = root.path().join("recordings");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();

        let linked = save(&link, &sample()).unwrap_err();
        let relative = save(Path::new("recordings"), &sample()).unwrap_err();

        let mode = fs::metadata(&elsewhere).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        // O_NOFOLLOW with O_DIRECTORY: a symlink is "not a directory".
        assert_eq!(linked.raw_os_error(), Some(libc::ENOTDIR), "{linked}");
        assert_eq!(relative.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_directory_replaced_after_its_check_never_receives_the_file() {
        let root = TempDir::new();
        let dir = root.path().join("recordings");
        let checked = private_dir(&dir).unwrap();
        // Moved away and replaced by a symlink to somewhere else.
        let moved = root.path().join("moved");
        let elsewhere = root.path().join("elsewhere");
        fs::rename(&dir, &moved).unwrap();
        fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();

        let name = save_in(&checked, &sample()).unwrap();

        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        let saved = fs::read(moved.join(&name)).unwrap();
        assert_eq!(Recording::from_json(&saved).unwrap(), sample());
        assert_eq!(fs::read_dir(&moved).unwrap().count(), 1);
    }

    #[test]
    fn a_second_recording_of_the_same_moment_gets_its_own_file() {
        let dir = TempDir::new();
        let recording = sample();
        let first = save(dir.path(), &recording).unwrap();
        let before = fs::read(&first).unwrap();

        let second = save(dir.path(), &recording).unwrap();

        assert_eq!(second, dir.path().join("2026-11-09_18-04-05.123-1.json"));
        assert_eq!(fs::read(&first).unwrap(), before);
    }
}
