//! Where recordings are kept: `$XDG_DATA_HOME/auto-clicker/recordings/`, one
//! file per recording, named after the time it was made.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use super::Recording;
use crate::fs::{Folder, STALE_TEMP_AGE, remove_stale_temps};

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

/// Saves `recording` in `dir`, creating the directory (private to its
/// owner) if needed, as a new file named after its creation time. Never
/// replaces a file. Returns the file's path, as `dir` named it.
///
/// The directory is opened and checked once, and the file is written,
/// put in place and flushed through that open directory, so a directory
/// renamed or replaced meanwhile never receives it ([`Folder`]).
///
/// # Errors
/// `dir` is relative, is a link, cannot be created or made private;
/// the file cannot be written; or every name tried is taken.
pub fn save(dir: &Path, recording: &Recording) -> io::Result<PathBuf> {
    let checked = Folder::open_private(dir)?;
    let name = save_in(&checked, recording).map_err(|err| checked.named(dir, &err))?;
    Ok(dir.join(name))
}

/// Saves `recording` in the open folder `checked`; the file's name.
fn save_in(checked: &Folder, recording: &Recording) -> io::Result<String> {
    let stem = recording.created().file_stem();
    let names: Vec<String> = (0..NAMES_TRIED)
        .map(|n| match n {
            0 => format!("{stem}.json"),
            n => format!("{stem}-{n}.json"),
        })
        .collect();
    checked.create_new(&names, |out| recording.write_json(out))
}

/// Removes the temporary files that saves in `dir` left when they crashed
/// at least [`STALE_TEMP_AGE`] ago; how many it removed.
///
/// # Errors
/// The folder cannot be read.
pub fn remove_stale(dir: &Path) -> io::Result<usize> {
    remove_stale_temps(dir, STALE_TEMP_AGE)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::recording::tests::sample;
    use crate::test_support::TempDir;

    #[cfg(unix)]
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
    fn saves_a_file_in_a_new_directory_and_reads_back() {
        let root = TempDir::new();
        let dir = root.path().join("auto-clicker").join("recordings");
        let recording = sample();

        let path = save(&dir, &recording).unwrap();

        assert_eq!(path, dir.join("2026-11-09_18-04-05.123.json"));
        let read = Recording::from_json(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(read, recording);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_its_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = TempDir::new();
        let dir = root.path().join("auto-clicker").join("recordings");

        let path = save(&dir, &sample()).unwrap();

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&dir), mode(&path)), (0o700, 0o600));
    }

    #[test]
    fn a_relative_path_is_refused() {
        let relative = save(Path::new("recordings"), &sample()).unwrap_err();
        assert_eq!(relative.kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_refused_and_nothing_changes() {
        use std::os::unix::fs::PermissionsExt;
        let root = TempDir::new();
        let elsewhere = root.path().join("project");
        fs::create_dir(&elsewhere).unwrap();
        fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o755)).unwrap();
        let link = root.path().join("recordings");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();

        let linked = save(&link, &sample()).unwrap_err();

        let mode = fs::metadata(&elsewhere).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        // O_NOFOLLOW with O_DIRECTORY: a symlink is "not a directory".
        assert_eq!(linked.raw_os_error(), Some(libc::ENOTDIR), "{linked}");
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_replaced_after_its_check_never_receives_the_file() {
        let root = TempDir::new();
        let dir = root.path().join("recordings");
        let checked = Folder::open_private(&dir).unwrap();
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
        for n in 2..10 {
            let taken = dir.path().join(format!("2026-11-09_18-04-05.123-{n}.json"));
            fs::write(taken, b"").unwrap();
        }
        let full = save(dir.path(), &recording).unwrap_err().to_string();
        let named = dir.path().join("2026-11-09_18-04-05.123.json");
        assert!(full.starts_with(&named.display().to_string()), "{full}");

        assert_eq!(second, dir.path().join("2026-11-09_18-04-05.123-1.json"));
        assert_eq!(fs::read(&first).unwrap(), before);
    }

    #[test]
    fn a_start_removes_only_old_temporary_files() {
        let dir = TempDir::new();
        let saved = save(dir.path(), &sample()).unwrap();
        let fresh = dir
            .path()
            .join(".2026-11-09_18-04-05.123.json.7.0123456789abcdef.tmp");
        fs::write(&fresh, b"half").unwrap();

        let removed = remove_stale(dir.path()).unwrap();

        assert_eq!(removed, 0);
        assert!(saved.exists() && fresh.exists());
    }
}
