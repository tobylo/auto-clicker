//! Where recordings are kept: `$XDG_DATA_HOME/auto-clicker/recordings/`, one
//! file per recording, named after the time it was made.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::{InvalidRecording, Recording};
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

/// The largest recording file that is read. An hour of fast mouse movement
/// is under 100 MiB; anything larger is not a recording this program made.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// The name of a recording's file, in the one form this program writes:
/// `2026-11-09_18-04-05.123.json`, or with `-1` to `-9` before the extension.
/// Nothing else in the folder is listed or opened, so a name is never a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileName(String);

impl FileName {
    /// `name` when it has a recording file's form.
    pub fn parse(name: &str) -> Option<FileName> {
        let stem = name.strip_suffix(".json")?;
        let (time, copy) = match stem.split_at_checked(TIME_FORM.len())? {
            (time, "") => (time, None),
            (time, copy) => (time, Some(copy)),
        };
        let digit_or_same = |(form, found): (u8, u8)| match form {
            b'0' => found.is_ascii_digit(),
            _ => found == form,
        };
        let copy_fits = copy
            .is_none_or(|copy| matches!(copy.as_bytes(), [b'-', n] if (b'1'..=b'9').contains(n)));
        (TIME_FORM.bytes().zip(time.bytes()).all(digit_or_same) && copy_fits)
            .then(|| FileName(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// What orders two names by when their recordings were made: the time,
    /// then the plain name before `-1` to `-9`.
    fn made(&self) -> (&str, usize, &str) {
        let (time, rest) = self.0.split_at(TIME_FORM.len());
        (time, rest.len(), rest)
    }

    /// What the tray calls the recording: "Recording 2026-11-09 18:04:05",
    /// with " (2)" for the file ending in `-1`, the second made in the same
    /// millisecond.
    pub fn label(&self) -> String {
        let (date, rest) = self.0.split_at(DATE_LEN);
        let time = rest[1..TIME_LEN].replace('-', ":");
        let copy = match rest.as_bytes()[SUFFIX_AT..] {
            [b'-', n, ..] => format!(" ({})", n - b'0' + 1),
            _ => String::new(),
        };
        format!("Recording {date} {time}{copy}")
    }
}

impl fmt::Display for FileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A recording file's name before `.json` and any `-N`: a `0` stands for a
/// digit, anything else for itself ([`Created::file_stem`](super::Created)).
const TIME_FORM: &str = "0000-00-00_00-00-00.000";

/// The length of the date in [`TIME_FORM`].
const DATE_LEN: usize = "0000-00-00".len();

/// After the date: the length of `_00-00-00`, the time to the second.
const TIME_LEN: usize = "_00-00-00".len();

/// After the date: where `.json` or `-N` begins.
const SUFFIX_AT: usize = TIME_FORM.len() - DATE_LEN;

/// Why a recording could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// There is no such file, or no recordings folder.
    Missing,
    /// The file or its folder could not be opened or read, or is something
    /// other than a file.
    Unreadable(io::Error),
    /// The file is larger than [`MAX_FILE_BYTES`].
    TooLarge,
    /// The file is not a valid recording.
    Invalid(InvalidRecording),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Missing => f.write_str("the file is no longer there"),
            LoadError::Unreadable(err) => write!(f, "the file could not be read: {err}"),
            LoadError::TooLarge => write!(
                f,
                "the file is larger than {} MiB",
                MAX_FILE_BYTES / (1024 * 1024)
            ),
            LoadError::Invalid(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// The newest recordings in `dir`, newest first, at most `limit`: the files
/// with a recording's name, by name, which is the time each was made. A
/// folder that does not exist yet holds none.
///
/// # Errors
/// The folder is a symlink or cannot be read.
pub fn list(dir: &Path, limit: usize) -> io::Result<Vec<FileName>> {
    let folder = match open_folder(dir) {
        Ok(folder) => folder,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut names = Vec::new();
    for entry in fs::read_dir(held_path(&folder))? {
        let name = entry?.file_name();
        names.extend(name.to_str().and_then(FileName::parse));
    }
    names.sort_unstable_by(|a, b| b.made().cmp(&a.made()));
    names.truncate(limit);
    Ok(names)
}

/// Reads and checks the recording `name` in `dir`. The file is opened once,
/// and what was opened is what is checked and read: a symlink, a FIFO or a
/// folder under that name is refused without waiting on it.
///
/// # Errors
/// See [`LoadError`].
pub fn load(dir: &Path, name: &FileName) -> Result<Recording, LoadError> {
    let missing_or = |err: io::Error| match err.kind() {
        io::ErrorKind::NotFound => LoadError::Missing,
        _ => LoadError::Unreadable(err),
    };
    let folder = open_folder(dir).map_err(missing_or)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(held_path(&folder).join(name.as_str()))
        .map_err(missing_or)?;
    let bytes = read_capped(file, MAX_FILE_BYTES)?;
    Recording::from_json(&bytes).map_err(LoadError::Invalid)
}

/// Everything in `file`, which must be a regular file of at most `cap`
/// bytes. The cap holds for what is read, whatever size the file claims.
fn read_capped(file: File, cap: u64) -> Result<Vec<u8>, LoadError> {
    let metadata = file.metadata().map_err(LoadError::Unreadable)?;
    if !metadata.is_file() {
        return Err(LoadError::Unreadable(io::Error::other("it is not a file")));
    }
    if metadata.len() > cap {
        return Err(LoadError::TooLarge);
    }
    let mut bytes = Vec::new();
    let read = file
        .take(cap.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(LoadError::Unreadable)?;
    match u64::try_from(read) {
        Ok(read) if read <= cap => Ok(bytes),
        _ => Err(LoadError::TooLarge),
    }
}

/// Opens the recordings folder for reading, refusing a symlink in its place.
fn open_folder(dir: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)
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
    let name = save_in(&checked, recording).map_err(|err| named(&checked, dir, &err))?;
    Ok(dir.join(name))
}

/// `err`, naming the folder where it is now, as the kernel sees the open
/// directory `checked`, rather than by its handle; as `dir` if that fails.
fn named(checked: &File, dir: &Path, err: &io::Error) -> io::Error {
    let held = held_path(checked);
    let now_at = fs::read_link(&held).unwrap_or_else(|_| dir.to_path_buf());
    let message = err
        .to_string()
        .replace(&held.display().to_string(), &now_at.display().to_string());
    io::Error::new(err.kind(), message)
}

/// A path the kernel resolves through the open directory `checked`, not
/// through its name.
fn held_path(checked: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", checked.as_raw_fd()))
}

/// Saves `recording` in the open directory `checked`; the file's name.
fn save_in(checked: &File, recording: &Recording) -> io::Result<OsString> {
    let held = held_path(checked);
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
    let handle = open_folder(dir)?;
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
    fn an_error_names_the_checked_folder_where_it_is_now() {
        let root = TempDir::new();
        let dir = root.path().join("recordings");
        let checked = private_dir(&dir).unwrap();
        let moved = root.path().join("moved");
        fs::rename(&dir, &moved).unwrap();
        let held = held_path(&checked).join("x.json");
        let err = io::Error::other(format!("{} is taken", held.display()));

        let shown = named(&checked, &dir, &err).to_string();

        let expected = fs::canonicalize(&moved).unwrap().join("x.json");
        assert_eq!(shown, format!("{} is taken", expected.display()));
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

    // Listing and loading

    fn name(text: &str) -> FileName {
        FileName::parse(text).unwrap()
    }

    #[test]
    fn only_the_names_this_program_writes_are_file_names() {
        let valid = [
            "2026-11-09_18-04-05.123.json",
            "2026-11-09_18-04-05.123-1.json",
            "2026-11-09_18-04-05.123-9.json",
            "0000-00-00_00-00-00.000.json",
        ];
        for text in valid {
            assert_eq!(
                FileName::parse(text)
                    .map(|name| name.to_string())
                    .as_deref(),
                Some(text)
            );
        }
        let invalid = [
            "",
            ".json",
            "2026-11-09_18-04-05.123",
            "2026-11-09_18-04-05.123.JSON",
            "2026-11-09_18-04-05.123.json.tmp",
            "2026-11-09_18-04-05.123-0.json",
            "2026-11-09_18-04-05.123-10.json",
            "2026-11-09_18-04-05.123-.json",
            "2026-11-09_18-04-05.123_1.json",
            "2026-11-09_18-04-05.12.json",
            "2026-11-09 18-04-05.123.json",
            "2026-11-09_18:04:05.123.json",
            "2026-11-0x_18-04-05.123.json",
            "../2026-11-09_18-04-05.123.json",
            "2026-11-09_18-04-05.123.json/",
            "mining.json",
            "²026-11-09_18-04-05.123.json",
            "2026-11-09_18-04-05.12³.json",
        ];
        for text in invalid {
            assert_eq!(FileName::parse(text), None, "{text:?}");
        }
        // What a save writes is a file name.
        assert!(FileName::parse(&format!("{}.json", sample().created().file_stem())).is_some());
    }

    #[test]
    fn a_label_gives_the_time_to_the_second_and_numbers_a_copy() {
        assert_eq!(
            name("2026-11-09_18-04-05.123.json").label(),
            "Recording 2026-11-09 18:04:05"
        );
        assert_eq!(
            name("2026-11-09_18-04-05.123-1.json").label(),
            "Recording 2026-11-09 18:04:05 (2)"
        );
        assert_eq!(
            name("2026-11-09_18-04-05.123-9.json").label(),
            "Recording 2026-11-09 18:04:05 (10)"
        );
    }

    #[test]
    fn lists_the_newest_recordings_first_and_nothing_else() {
        let dir = TempDir::new();
        let names = [
            "2026-11-09_18-04-05.123.json",
            "2026-11-09_18-04-05.123-1.json",
            "2026-11-09_18-04-05.123-2.json",
            "2026-11-10_07-00-00.000.json",
            "2025-01-01_00-00-00.000.json",
        ];
        for file in names {
            fs::write(dir.path().join(file), b"").unwrap();
        }
        for other in [
            "notes.txt",
            "mining.json",
            ".2026-11-09_18-04-05.123.json.tmp",
        ] {
            fs::write(dir.path().join(other), b"").unwrap();
        }
        // Whatever has a recording's name is listed; a load refuses what is
        // not a file.
        fs::create_dir(dir.path().join("2024-01-01_00-00-00.000.json")).unwrap();

        let listed = |limit| -> Vec<String> {
            let found = list(dir.path(), limit).unwrap();
            found.iter().map(ToString::to_string).collect()
        };

        let newest_first = [
            "2026-11-10_07-00-00.000.json",
            "2026-11-09_18-04-05.123-2.json",
            "2026-11-09_18-04-05.123-1.json",
            "2026-11-09_18-04-05.123.json",
            "2025-01-01_00-00-00.000.json",
            "2024-01-01_00-00-00.000.json",
        ];
        assert_eq!(listed(10), newest_first);
        assert_eq!(listed(2), newest_first[..2]);
        assert_eq!(listed(0), [""; 0]);
    }

    #[test]
    fn a_missing_folder_lists_nothing_and_a_symlink_is_an_error() {
        let root = TempDir::new();
        assert_eq!(list(&root.path().join("none"), 10).unwrap(), []);

        let real = root.path().join("real");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("2026-11-09_18-04-05.123.json"), b"").unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let err = list(&link, 10).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::ENOTDIR), "{err}");
        // A file in the folder's place is an error too, not an empty list.
        let err = list(&real.join("2026-11-09_18-04-05.123.json"), 10).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::ENOTDIR), "{err}");
    }

    #[test]
    fn loads_what_was_saved() {
        let dir = TempDir::new();
        let path = save(dir.path(), &sample()).unwrap();
        let file = name(path.file_name().unwrap().to_str().unwrap());
        assert_eq!(load(dir.path(), &file).unwrap(), sample());
    }

    #[test]
    fn a_missing_file_or_folder_is_missing() {
        let dir = TempDir::new();
        let file = name("2026-11-09_18-04-05.123.json");
        let missing = |dir: &Path| matches!(load(dir, &file), Err(LoadError::Missing));
        assert!(missing(dir.path()));
        assert!(missing(&dir.path().join("none")));
        assert_eq!(
            LoadError::Missing.to_string(),
            "the file is no longer there"
        );
    }

    #[test]
    fn an_invalid_file_says_why_without_what_it_holds() {
        let dir = TempDir::new();
        let file = name("2026-11-09_18-04-05.123.json");
        fs::write(dir.path().join(file.as_str()), b"{\"version\":1,\"KEY_W\"").unwrap();
        let err = load(dir.path(), &file).unwrap_err();
        assert!(matches!(err, LoadError::Invalid(_)), "{err:?}");
        assert_eq!(
            err.to_string(),
            "the file ends too early (line 1, column 20)"
        );
    }

    #[test]
    fn a_symlink_a_fifo_or_a_folder_in_a_files_place_is_refused_at_once() {
        let dir = TempDir::new();
        let saved = save(dir.path(), &sample()).unwrap();
        let link = name("2026-01-01_00-00-00.000.json");
        std::os::unix::fs::symlink(&saved, dir.path().join(link.as_str())).unwrap();
        let fifo = name("2026-01-02_00-00-00.000.json");
        let made = std::process::Command::new("mkfifo")
            .arg(dir.path().join(fifo.as_str()))
            .status()
            .unwrap();
        assert!(made.success());
        let folder = name("2026-01-03_00-00-00.000.json");
        fs::create_dir(dir.path().join(folder.as_str())).unwrap();

        for file in [link, fifo, folder] {
            let err = load(dir.path(), &file).unwrap_err();
            assert!(matches!(err, LoadError::Unreadable(_)), "{file}: {err:?}");
        }
        // The folder itself may not be a symlink either.
        let elsewhere = TempDir::new();
        let linked = elsewhere.path().join("recordings");
        std::os::unix::fs::symlink(dir.path(), &linked).unwrap();
        let file = name(saved.file_name().unwrap().to_str().unwrap());
        let err = load(&linked, &file).unwrap_err();
        assert!(matches!(err, LoadError::Unreadable(_)), "{err:?}");
    }

    #[test]
    fn a_file_over_the_cap_is_refused_whatever_size_it_claims() {
        let dir = TempDir::new();
        let path = dir.path().join("file");
        fs::write(&path, b"12345").unwrap();
        let read = |cap| read_capped(File::open(&path).unwrap(), cap);
        assert_eq!(read(5).unwrap(), b"12345");
        assert!(matches!(read(4), Err(LoadError::TooLarge)));
        assert_eq!(
            LoadError::TooLarge.to_string(),
            "the file is larger than 256 MiB"
        );
        // The cap holds for what is read: procfs files claim a size of 0.
        let status = File::open("/proc/self/status").unwrap();
        assert_eq!(status.metadata().unwrap().len(), 0);
        assert!(matches!(read_capped(status, 4), Err(LoadError::TooLarge)));
        // Not a regular file.
        let zero = File::open("/dev/zero").unwrap();
        assert!(matches!(
            read_capped(zero, 4),
            Err(LoadError::Unreadable(_))
        ));
    }
}
