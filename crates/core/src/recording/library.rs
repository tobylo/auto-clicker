//! The recordings that can be played: which files there are, with the name
//! each holds, and reading one.
//!
//! Files are untrusted: a name is taken in one form only, and a file is
//! opened once, checked as opened, read up to a cap and parsed whole by
//! [`Recording::from_json`].

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::header;
use super::store::{held_path, open_folder};
use super::{InvalidRecording, Recording};

/// The largest recording file that is read. An hour of fast mouse movement
/// is under 110 MB, and about 245 MB with a position each millisecond; the
/// recorder stops a recording before its file would grow past this, so
/// anything larger is not a recording this program made.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// The name of a recording's file, in the one form this program writes:
/// `2026-11-09_18-04-05.123.json`, or with `-1` to `-9` before the extension.
/// Nothing else in the folder is listed or opened, so a name is never a path.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct FileName(String);

impl TryFrom<String> for FileName {
    type Error = String;

    fn try_from(name: String) -> Result<FileName, String> {
        FileName::parse(&name).ok_or_else(|| "not a recording's file name".to_owned())
    }
}

impl From<FileName> for String {
    fn from(name: FileName) -> String {
        name.0
    }
}

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

    /// What the tray calls the recording when it has the name it was made
    /// with: "Recording 2026-11-09 18:04:05", with " (2)" for the file
    /// ending in `-1`, the second made in the same millisecond.
    pub fn label(&self) -> String {
        format!("Recording {}", self.when())
    }

    /// When the recording was made, to the second, as the lists show it:
    /// "2026-11-09 18:04:05", with " (2)" for the file ending in `-1`.
    pub fn when(&self) -> String {
        let (date, rest) = self.0.split_at(DATE_LEN);
        let time = rest[1..TIME_LEN].replace('-', ":");
        let copy = match rest.as_bytes()[SUFFIX_AT..] {
            [b'-', n, ..] => format!(" ({})", n - b'0' + 1),
            _ => String::new(),
        };
        format!("{date} {time}{copy}")
    }

    /// What the lists call the recording named `name`: its label when it
    /// has the name a new recording gets (or none could be read), else the
    /// name with when it was made, "Farm loop (2026-11-09 18:04:05)".
    pub fn titled(&self, name: Option<&str>) -> String {
        match name {
            Some(name) if name != self.default_name() => format!("{name} ({})", self.when()),
            _ => self.label(),
        }
    }

    /// The name a recording made at this file's time gets:
    /// "Recording 2026-11-09 18:04".
    fn default_name(&self) -> String {
        let (date, rest) = self.0.split_at(DATE_LEN);
        format!("Recording {date} {}", rest[1..MINUTE_LEN].replace('-', ":"))
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

/// After the date: the length of `_00-00`, the time to the minute.
const MINUTE_LEN: usize = "_00-00".len();

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

/// A recording found in the folder: its file, and the name its header
/// holds; `None` when the start of the file does not read as a recording's
/// header (see [`header`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub file: FileName,
    pub name: Option<String>,
}

/// The newest recordings in a folder, and how many it holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// Newest first.
    pub newest: Vec<Found>,
    /// Every file with a recording's name, listed or not.
    pub total: usize,
}

/// The newest recordings in `dir`, newest first, at most `limit`: the files
/// with a recording's name, by name, which is the local time each was made
/// (in the hour the clocks go back, that order can differ from the true
/// one). Only the start of each listed file is read, for its name. A folder
/// that does not exist yet holds none.
///
/// # Errors
/// The folder is a symlink or cannot be read.
pub fn list(dir: &Path, limit: usize) -> io::Result<Listing> {
    let folder = match open_folder(dir) {
        Ok(folder) => folder,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Listing::default()),
        Err(err) => return Err(err),
    };
    let mut newest = Vec::new();
    let mut total: usize = 0;
    for entry in fs::read_dir(held_path(&folder))? {
        let name = entry?.file_name();
        let Some(file) = name.to_str().and_then(FileName::parse) else {
            continue;
        };
        total = total.saturating_add(1);
        newest.push(file);
        // However many files there are, only about `limit` names are kept.
        if newest.len() >= limit.saturating_mul(2).saturating_add(64) {
            keep_newest(&mut newest, limit);
        }
    }
    keep_newest(&mut newest, limit);
    let newest = newest
        .into_iter()
        .map(|file| Found {
            name: header::name(&folder, &file),
            file,
        })
        .collect();
    Ok(Listing { newest, total })
}

/// Sorts `names` newest first and drops all but the first `limit`.
fn keep_newest(names: &mut Vec<FileName>, limit: usize) {
    names.sort_unstable_by(|a, b| b.made().cmp(&a.made()));
    names.truncate(limit);
}

/// Reads and checks the recording `name` in `dir`. The file is opened once,
/// and what was opened is what is checked and read: a symlink, a FIFO or a
/// folder under that name is refused without waiting on it.
///
/// # Errors
/// See [`LoadError`].
pub fn load(dir: &Path, name: &FileName) -> Result<Recording, LoadError> {
    let folder = open_folder(dir).map_err(missing_or)?;
    load_in(&folder, name)
}

/// [`load`] from the open recordings folder `folder`.
pub(super) fn load_in(folder: &File, name: &FileName) -> Result<Recording, LoadError> {
    let file = open_in(folder, name).map_err(missing_or)?;
    let bytes = read_capped(file, MAX_FILE_BYTES)?;
    Recording::from_json(&bytes).map_err(LoadError::Invalid)
}

/// `err` as a load error: a file or folder that is not there is missing.
fn missing_or(err: io::Error) -> LoadError {
    match err.kind() {
        io::ErrorKind::NotFound => LoadError::Missing,
        _ => LoadError::Unreadable(err),
    }
}

/// Opens the file `name` in the open folder `folder` for reading, without
/// following a symlink and without waiting on a FIFO.
pub(super) fn open_in(folder: &File, name: &FileName) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(held_path(folder).join(name.as_str()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::save;
    use crate::recording::tests::sample;
    use crate::test_support::TempDir;

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
    fn a_title_shows_a_name_with_its_time_unless_it_is_the_name_it_was_made_with() {
        let plain = name("2026-11-09_18-04-05.123.json");
        let copy = name("2026-11-09_18-04-05.123-1.json");
        assert_eq!(plain.when(), "2026-11-09 18:04:05");
        assert_eq!(copy.when(), "2026-11-09 18:04:05 (2)");
        let cases = [
            (&plain, Some("Farm loop"), "Farm loop (2026-11-09 18:04:05)"),
            (
                &copy,
                Some("Farm loop"),
                "Farm loop (2026-11-09 18:04:05 (2))",
            ),
            (&plain, None, "Recording 2026-11-09 18:04:05"),
            // The name a recording made then gets.
            (
                &plain,
                Some("Recording 2026-11-09 18:04"),
                "Recording 2026-11-09 18:04:05",
            ),
            (
                &copy,
                Some("Recording 2026-11-09 18:04"),
                "Recording 2026-11-09 18:04:05 (2)",
            ),
            // Another minute's default name is a name like any other.
            (
                &plain,
                Some("Recording 2026-11-09 18:05"),
                "Recording 2026-11-09 18:05 (2026-11-09 18:04:05)",
            ),
        ];
        for (file, named, title) in cases {
            assert_eq!(file.titled(named), title, "{file} {named:?}");
        }
        let made = sample();
        let file = name(&format!("{}.json", made.created().file_stem()));
        assert_eq!(file.titled(Some(made.name())), file.label());
    }

    #[test]
    fn a_listing_reads_each_listed_files_name_and_no_other_file() {
        let dir = TempDir::new();
        let saved = save(dir.path(), &sample()).unwrap();
        let file = name(saved.file_name().unwrap().to_str().unwrap());
        // Older than the limit lets in: its start is never read.
        fs::write(dir.path().join("2020-01-01_00-00-00.000.json"), b"").unwrap();
        // A file that does not start as a recording has no name.
        fs::write(dir.path().join("2030-01-01_00-00-00.000.json"), b"[]").unwrap();
        let found = list(dir.path(), 2).unwrap();
        assert_eq!(found.total, 3);
        assert_eq!(
            found.newest,
            [
                Found {
                    file: name("2030-01-01_00-00-00.000.json"),
                    name: None,
                },
                Found {
                    file,
                    name: Some(sample().name().to_owned()),
                },
            ]
        );
    }

    #[test]
    fn a_file_name_is_read_from_json_only_in_its_one_form() {
        let read = |text: &str| serde_json::from_str::<FileName>(text).ok();
        assert_eq!(
            read("\"2026-11-09_18-04-05.123.json\""),
            Some(name("2026-11-09_18-04-05.123.json"))
        );
        for text in [
            "\"../2026-11-09_18-04-05.123.json\"",
            "\"x.json\"",
            "7",
            "null",
        ] {
            assert_eq!(read(text), None, "{text}");
        }
        let written = serde_json::to_string(&name("2026-11-09_18-04-05.123-1.json")).unwrap();
        assert_eq!(written, "\"2026-11-09_18-04-05.123-1.json\"");
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
            assert_eq!(found.total, 6);
            found
                .newest
                .iter()
                .map(|found| found.file.to_string())
                .collect()
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
    fn a_folder_of_many_recordings_lists_only_its_newest() {
        let dir = TempDir::new();
        // Written in an order that is neither oldest nor newest first.
        for n in (0..500).map(|n| (n * 37) % 500) {
            let file = format!("2026-11-09_18-{:02}-{:02}.000.json", n / 60, n % 60);
            fs::write(dir.path().join(file), b"").unwrap();
        }
        let found = list(dir.path(), 3).unwrap();
        assert_eq!(found.total, 500);
        let listed: Vec<String> = found
            .newest
            .iter()
            .map(|found| found.file.to_string())
            .collect();
        let newest = ["08-19", "08-18", "08-17"].map(|at| format!("2026-11-09_18-{at}.000.json"));
        assert_eq!(listed, newest);
    }

    #[test]
    fn a_missing_folder_lists_nothing_and_a_symlink_is_an_error() {
        let root = TempDir::new();
        assert_eq!(
            list(&root.path().join("none"), 10).unwrap(),
            Listing::default()
        );

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
