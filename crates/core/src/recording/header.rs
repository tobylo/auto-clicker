//! A recording's name, read from the start of its file, so a list can show
//! it without reading the whole file.
//!
//! Only the header this program writes is read: the file starts with
//! `{"version":1,"name":` (or version 2 or 3), then the name as a JSON string,
//! which must pass the same check a whole recording's name does. Anything
//! else has no name to show; a load still reads the whole file as before.

use std::fs::File;
use std::io::Read;

use super::library::{FileName, open_in};
use super::{VERSION, VERSION_HYPRLAND, VERSION_POSITIONS, check_name};

/// How much of a file is read for its name. The name is at most 200
/// characters; written as `\u` escapes of surrogate pairs, the longest
/// spelling of one, that is 2,400 bytes after a header of 20.
const HEADER_BYTES: u64 = 4096;

/// The name in the header of the recording `file` in the open folder
/// `folder`; `None` when the file is not a regular file, cannot be read,
/// or does not start as a recording this program writes.
pub(super) fn name(folder: &File, file: &FileName) -> Option<String> {
    let opened = open_in(folder, file).ok()?;
    if !opened.metadata().ok()?.is_file() {
        return None;
    }
    let mut start = Vec::new();
    opened.take(HEADER_BYTES).read_to_end(&mut start).ok()?;
    parse(&start)
}

/// The name in `start`, the first bytes of a recording file.
fn parse(start: &[u8]) -> Option<String> {
    let rest = [VERSION, VERSION_POSITIONS, VERSION_HYPRLAND]
        .into_iter()
        .find_map(|version| {
            let prefix = format!("{{\"version\":{version},\"name\":");
            start.strip_prefix(prefix.as_bytes())
        })?;
    // One JSON string, which ends at its closing quote: what follows it is
    // never read.
    let name: String = serde_json::Deserializer::from_slice(rest)
        .into_iter()
        .next()?
        .ok()?;
    check_name(&name).ok()?;
    Some(name)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::recording::store::open_folder;
    use crate::recording::tests::sample;
    use crate::recording::{VERSION, save};
    use crate::test_support::TempDir;

    #[test]
    fn the_name_is_read_from_the_header_this_program_writes() {
        let cases = [
            (
                r#"{"version":1,"name":"Farm loop","created""#,
                Some("Farm loop"),
            ),
            (
                r#"{"version":2,"name":"Farm \"loop\"","#,
                Some("Farm \"loop\""),
            ),
            (r#"{"version":3,"name":"Farm","#, Some("Farm")),
            (r#"{"version":1,"name":"åker","#, Some("åker")),
            // A name cut short has none; what follows a whole one is never
            // read.
            (r#"{"version":1,"name":"Farm"#, None),
            (r#"{"version":1,"name":"Farm",]]"#, Some("Farm")),
            // Another layout, version or spelling has no name to show.
            (r#"{"name":"Farm","version":1,"#, None),
            (r#"{"version":4,"name":"Farm","#, None),
            (r#"{ "version":1,"name":"Farm","#, None),
            (r#"{"version":1,"name":7,"#, None),
            // A name a recording may not have.
            (r#"{"version":1,"name":"  ","#, None),
            (r#"{"version":1,"name":"a\nb","#, None),
            ("", None),
        ];
        for (start, expected) in cases {
            assert_eq!(parse(start.as_bytes()).as_deref(), expected, "{start}");
        }
        let long = format!(r#"{{"version":{VERSION},"name":"{}","#, "x".repeat(201));
        assert_eq!(parse(long.as_bytes()), None);
    }

    #[test]
    fn a_saved_recording_shows_its_name_and_anything_else_none() {
        let dir = TempDir::new();
        let saved = save(dir.path(), &sample()).unwrap();
        let file = FileName::parse(saved.file_name().unwrap().to_str().unwrap()).unwrap();
        let folder = open_folder(dir.path()).unwrap();
        assert_eq!(name(&folder, &file).as_deref(), Some(sample().name()));

        let link = FileName::parse("2026-01-01_00-00-00.000.json").unwrap();
        std::os::unix::fs::symlink(&saved, dir.path().join(link.as_str())).unwrap();
        let folder_in_place = FileName::parse("2026-01-02_00-00-00.000.json").unwrap();
        fs::create_dir(dir.path().join(folder_in_place.as_str())).unwrap();
        let fifo = FileName::parse("2026-01-03_00-00-00.000.json").unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(dir.path().join(fifo.as_str()))
            .status()
            .unwrap();
        assert!(made.success());
        let missing = FileName::parse("2026-01-04_00-00-00.000.json").unwrap();
        for file in [link, folder_in_place, fifo, missing] {
            // At once: a FIFO is never waited on.
            let held = folder.try_clone().unwrap();
            let (send, received) = std::sync::mpsc::channel();
            let asked = file.clone();
            std::thread::spawn(move || send.send(name(&held, &asked)));
            let read = received.recv_timeout(std::time::Duration::from_secs(5));
            assert_eq!(read.expect("the read waited"), None, "{file}");
        }
    }
}
