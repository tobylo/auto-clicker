//! Where recordings are kept: `$XDG_DATA_HOME/auto-clicker/recordings/`, one
//! file per recording, named after the time it was made.

use std::ffi::OsString;
use std::fs::DirBuilder;
use std::io;
use std::os::unix::fs::DirBuilderExt;
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
/// replaces a file. Returns the file's path.
///
/// # Errors
/// The directory cannot be created, the file cannot be written, or every
/// name tried is taken.
pub fn save(dir: &Path, recording: &Recording) -> io::Result<PathBuf> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let stem = recording.created().file_stem();
    let paths: Vec<PathBuf> = (0..NAMES_TRIED)
        .map(|n| match n {
            0 => dir.join(format!("{stem}.json")),
            n => dir.join(format!("{stem}-{n}.json")),
        })
        .collect();
    atomic_create(&paths, |out| recording.write_json(out))
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
