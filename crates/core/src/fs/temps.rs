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

/// The name of the file the temporary file `name` was written for, if
/// `name` is one [`temp_name`] makes.
pub(super) fn target_of(name: &str) -> Option<&str> {
    let inner = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let (rest, random) = inner.rsplit_once('.')?;
    let (target, pid) = rest.rsplit_once('.')?;
    let valid = !target.is_empty() && is_pid(pid) && is_random(random);
    valid.then_some(target)
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

/// A temporary file a save left in a folder.
#[derive(Debug)]
pub struct Temp {
    name: String,
    target: String,
    changed_ago: Option<Duration>,
}

impl Temp {
    /// Its file name in the folder.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The name of the file it was written for.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Whether it was last changed at least `age` ago. A time in the future
    /// is not.
    fn is_older_than(&self, age: Duration) -> bool {
        self.changed_ago.is_some_and(|ago| ago >= age)
    }
}

/// The temporary files in `dir`: the regular files (not links) named as
/// [`temp_name`] names them.
///
/// # Errors
/// The folder, or an entry in it, cannot be read: never taken as "none".
pub(super) fn list(dir: &Path) -> io::Result<Vec<Temp>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        let Some(target) = target_of(&name).map(ToOwned::to_owned) else {
            continue;
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            // Removed since the folder was read: nothing to remove.
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        if metadata.file_type().is_file() {
            let changed_ago = metadata.modified().ok().and_then(|at| at.elapsed().ok());
            found.push(Temp {
                name,
                target,
                changed_ago,
            });
        }
    }
    Ok(found)
}

/// Removes the temporary files in `dir` that are at least `age` old: what
/// a save that crashed left. A missing folder holds none. Returns how many
/// it removed.
///
/// # Errors
/// The folder cannot be read, or a file could not be removed (the first
/// such error; the others are still tried).
pub fn remove_stale_temps(dir: &Path, age: Duration) -> io::Result<usize> {
    let remove = |path: &Path| fs::remove_file(path);
    match remove_stale_in(dir, age, remove) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(0),
        found => found,
    }
}

/// Removes, with `remove`, the temporary files in `dir` last changed at
/// least `age` ago. Returns how many it removed, or the first failure
/// after trying them all; one already gone is no failure.
pub(super) fn remove_stale_in(
    dir: &Path,
    age: Duration,
    remove: impl Fn(&Path) -> io::Result<()>,
) -> io::Result<usize> {
    let mut removed: usize = 0;
    let mut failure = None;
    for temp in list(dir)?.iter().filter(|temp| temp.is_older_than(age)) {
        let path = dir.join(temp.name());
        match remove(&path) {
            Ok(()) => removed = removed.saturating_add(1),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                failure.get_or_insert_with(|| not_removed(&path, &err));
            }
        }
    }
    failure.map_or(Ok(removed), Err)
}

/// `err`, saying which temporary file it kept.
pub(super) fn not_removed(path: &Path, err: &io::Error) -> io::Error {
    io::Error::new(
        err.kind(),
        format!("could not remove {}: {err}", path.display()),
    )
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
        assert!(target_of(name).is_some(), "{name}");
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
            assert!(target_of(name).is_some(), "{name}");
        }
        for name in invalid {
            assert!(target_of(name).is_none(), "{name}");
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
    fn a_temp_file_that_cannot_be_removed_is_reported_and_one_already_gone_is_not() {
        let dir = TempDir::new();
        let names = [
            ".a.json.1.0123456789abcdef.tmp",
            ".b.json.1.0123456789abcdef.tmp",
            ".c.json.1.0123456789abcdef.tmp",
        ];
        for name in names {
            fs::write(dir.path().join(name), b"half").unwrap();
        }
        let held = dir.path().join(names[1]);
        // Removed by someone else meanwhile: no failure.
        let gone = dir.path().join(names[2]);

        let swept = remove_stale_in(dir.path(), Duration::ZERO, |path| {
            if path == held {
                return Err(io::Error::other("held"));
            }
            fs::remove_file(path)?;
            if path == gone {
                return Err(io::ErrorKind::NotFound.into());
            }
            Ok(())
        });

        let err = swept.unwrap_err();
        let text = err.to_string();
        assert!(text.starts_with("could not remove "), "{text}");
        assert!(text.ends_with(&format!("{}: held", names[1])), "{text}");
        assert_eq!(dir.entries(), [names[1]]);
    }

    #[test]
    fn a_temp_file_names_the_file_it_was_for() {
        let dir = TempDir::new();
        fs::write(dir.path().join(".a.b.json.7.0123456789abcdef.tmp"), b"").unwrap();
        fs::write(dir.path().join("a.b.json"), b"").unwrap();

        let temps = list(dir.path()).unwrap();

        let found: Vec<(&str, &str)> = temps.iter().map(|t| (t.name(), t.target())).collect();
        assert_eq!(found, [(".a.b.json.7.0123456789abcdef.tmp", "a.b.json")]);
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
