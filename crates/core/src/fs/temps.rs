//! Temporary files: how a save names them, and removing those a crash left.
//!
//! A save writes `.<target>.<process id>.<16 hex digits>.tmp` beside its
//! target. The leading dot hides it, and no recording's file name has that
//! form, so a listing never shows it. Only a name of exactly that form is
//! ever removed, so no file of the user's is.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

const PREFIX: &str = ".";
const SUFFIX: &str = ".tmp";
const RANDOM_DIGITS: usize = 16;

/// A new temporary file name for the target named `name`.
pub(super) fn temp_name(name: &OsStr) -> OsString {
    let mut temp = OsString::from(PREFIX);
    temp.push(name);
    temp.push(format!(
        ".{}.{:016x}{SUFFIX}",
        std::process::id(),
        fastrand::u64(..)
    ));
    temp
}

/// Whether `name` is one [`temp_name`] makes.
pub(super) fn is_temp_name(name: &str) -> bool {
    let inner = name
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_suffix(SUFFIX));
    let parts = inner.and_then(|inner| {
        let (rest, random) = inner.rsplit_once('.')?;
        let (target, pid) = rest.rsplit_once('.')?;
        Some((target, pid, random))
    });
    parts
        .is_some_and(|(target, pid, random)| !target.is_empty() && is_pid(pid) && is_random(random))
}

/// A process id: 1 to 10 decimal digits.
fn is_pid(text: &str) -> bool {
    (1..=10).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
}

/// What [`temp_name`] writes after the process id: 16 lowercase hex digits.
fn is_random(text: &str) -> bool {
    text.len() == RANDOM_DIGITS
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Removes the temporary files in `dir` that are at least `age` old: what
/// a save that crashed left. A missing folder holds none. Returns how many
/// it removed; one it cannot remove is left for the next start.
///
/// # Errors
/// The folder cannot be read.
pub fn remove_stale_temps(dir: &Path, age: Duration) -> io::Result<usize> {
    match remove_in(dir, age, |path| fs::remove_file(path)) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(0),
        found => found,
    }
}

/// Removes, with `remove`, the regular files in `dir` named as temporary
/// files and last changed at least `age` ago. Returns how many it removed.
pub(super) fn remove_in(
    dir: &Path,
    age: Duration,
    remove: impl Fn(&Path) -> io::Result<()>,
) -> io::Result<usize> {
    let mut removed: usize = 0;
    for entry in fs::read_dir(dir)?.flatten() {
        if is_stale_temp(&entry, age) && remove(&entry.path()).is_ok() {
            removed = removed.saturating_add(1);
        }
    }
    Ok(removed)
}

/// Whether `entry` is named as a temporary file and is stale.
fn is_stale_temp(entry: &fs::DirEntry, age: Duration) -> bool {
    let named = entry.file_name().to_str().is_some_and(is_temp_name);
    named && is_stale(entry, age)
}

/// Whether `entry` is a regular file (not a link) last changed at least
/// `age` ago. A time in the future is not stale.
fn is_stale(entry: &fs::DirEntry, age: Duration) -> bool {
    let Ok(metadata) = entry.metadata() else {
        return false;
    };
    let changed_ago = metadata.modified().ok().and_then(|at| at.elapsed().ok());
    metadata.file_type().is_file() && changed_ago.is_some_and(|ago| ago >= age)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn a_temp_name_is_hidden_and_reads_back_as_one() {
        let name = temp_name(OsStr::new("config.toml"));
        let name = name.to_str().unwrap();
        assert!(name.starts_with(".config.toml."), "{name}");
        assert!(is_temp_name(name), "{name}");
    }

    #[test]
    fn only_the_exact_form_is_a_temp_name() {
        let valid = [
            ".config.toml.42.0123456789abcdef.tmp",
            ".2026-11-09_18-04-05.123.json.1.ffffffffffffffff.tmp",
            ".x.4294967295.0000000000000000.tmp",
        ];
        let invalid = [
            "",
            "config.toml.42.0123456789abcdef.tmp",
            ".config.toml.42.0123456789abcdef",
            "..42.0123456789abcdef.tmp",
            ".config.toml..0123456789abcdef.tmp",
            ".config.toml.4x.0123456789abcdef.tmp",
            ".config.toml.12345678901.0123456789abcdef.tmp",
            ".config.toml.42.0123456789abcde.tmp",
            ".config.toml.42.0123456789abcdef0.tmp",
            ".config.toml.42.0123456789ABCDEF.tmp",
            ".config.toml.42.0123456789abcdeg.tmp",
            ".42.0123456789abcdef.tmp",
            ".config.toml.42.0123456789abcdef.TMP",
            ".notes.tmp",
        ];
        for name in valid {
            assert!(is_temp_name(name), "{name}");
        }
        for name in invalid {
            assert!(!is_temp_name(name), "{name}");
        }
    }

    #[test]
    fn only_old_temp_files_are_removed_at_a_start() {
        let dir = TempDir::new();
        let temp = dir.path().join(".config.toml.42.0123456789abcdef.tmp");
        fs::write(&temp, b"half").unwrap();
        fs::write(dir.path().join("config.toml"), b"kept").unwrap();
        fs::write(dir.path().join(".notes.tmp"), b"kept").unwrap();
        fs::create_dir(dir.path().join(".x.1.0123456789abcdef.tmp")).unwrap();

        assert_eq!(
            remove_stale_temps(dir.path(), Duration::from_secs(3600)).unwrap(),
            0
        );
        assert_eq!(remove_stale_temps(dir.path(), Duration::ZERO).unwrap(), 1);

        assert_eq!(
            dir.entries(),
            [".notes.tmp", ".x.1.0123456789abcdef.tmp", "config.toml"]
        );
    }

    #[test]
    fn a_temp_file_older_than_the_age_is_removed() {
        let dir = TempDir::new();
        let temp = dir.path().join(".config.toml.42.0123456789abcdef.tmp");
        fs::write(&temp, b"half").unwrap();
        let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        fs::File::options()
            .write(true)
            .open(&temp)
            .unwrap()
            .set_modified(two_hours_ago)
            .unwrap();

        let removed = remove_stale_temps(dir.path(), Duration::from_secs(60 * 60)).unwrap();

        assert_eq!(removed, 1);
        assert_eq!(dir.entries(), Vec::<String>::new());
    }

    #[test]
    fn a_missing_folder_holds_no_temp_files() {
        let dir = TempDir::new();
        let missing = dir.path().join("missing");
        assert_eq!(remove_stale_temps(&missing, Duration::ZERO).unwrap(), 0);
    }

    #[test]
    fn a_file_in_place_of_the_folder_is_an_error() {
        let dir = TempDir::new();
        let file = dir.path().join("file");
        fs::write(&file, b"").unwrap();
        assert!(remove_stale_temps(&file, Duration::ZERO).is_err());
    }
}
