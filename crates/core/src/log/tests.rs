use std::fs;

use super::*;
use crate::test_support::TempDir;

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

#[test]
fn lines_are_appended_to_what_the_file_holds_in_a_folder_made_for_it() {
    let dir = TempDir::new();
    let path = dir.path().join("logs").join("auto-clicker.log");
    fs::create_dir(dir.path().join("logs")).unwrap();
    fs::write(&path, "earlier\n").unwrap();
    let mut log = LogFile::open(path.clone(), 100).unwrap();
    log.write_line("now\n");
    assert_eq!(read(&path), "earlier\nnow\n");

    let fresh = dir
        .path()
        .join("new")
        .join("deeper")
        .join("auto-clicker.log");
    LogFile::open(fresh.clone(), 100)
        .unwrap()
        .write_line("first\n");
    assert_eq!(read(&fresh), "first\n");
}

#[test]
fn a_line_that_would_pass_the_cap_starts_a_new_file_and_keeps_one_old() {
    let dir = TempDir::new();
    let path = dir.path().join("auto-clicker.log");
    fs::write(&path, "0123456789\n").unwrap();
    let mut log = LogFile::open(path.clone(), 16).unwrap();
    // 11 + 5 is the cap exactly: still this file.
    log.write_line("abcd\n");
    assert_eq!(read(&path), "0123456789\nabcd\n");
    log.write_line("next\n");
    assert_eq!(read(&path), "next\n");
    assert_eq!(read(&rotated(&path)), "0123456789\nabcd\n");
    // The next rotation replaces the old `.1`.
    log.write_line("0123456789ab\n");
    assert_eq!(read(&path), "0123456789ab\n");
    assert_eq!(read(&rotated(&path)), "next\n");
    assert_eq!(dir.entries(), ["auto-clicker.log", "auto-clicker.log.1"]);
}

#[test]
fn a_full_file_that_cannot_be_moved_keeps_its_lines() {
    let dir = TempDir::new();
    let path = dir.path().join("auto-clicker.log");
    // A folder with something in it where `.1` would go: no rename replaces it.
    fs::create_dir(rotated(&path)).unwrap();
    fs::write(rotated(&path).join("held"), "").unwrap();
    let mut log = LogFile::open(path.clone(), 8).unwrap();
    log.write_line("one two\n");
    log.write_line("three\n");
    log.write_line("four\n");
    assert_eq!(read(&path), "one two\nthree\nfour\n");
    // Once the way is clear, the next line starts a new file.
    fs::remove_dir_all(rotated(&path)).unwrap();
    log.write_line("five\n");
    assert_eq!(read(&path), "five\n");
    assert_eq!(read(&rotated(&path)), "one two\nthree\nfour\n");
}

#[test]
fn a_log_file_that_cannot_be_opened_is_an_error() {
    let dir = TempDir::new();
    let blocker = dir.path().join("file");
    fs::write(&blocker, "").unwrap();
    assert!(LogFile::open(blocker.join("auto-clicker.log"), 8).is_err());
    assert!(LogFile::open(dir.path().to_owned(), 8).is_err());
}

#[test]
fn the_rotated_file_is_named_after_the_log() {
    assert_eq!(
        rotated(Path::new("logs/auto-clicker.log")),
        Path::new("logs/auto-clicker.log.1")
    );
}
