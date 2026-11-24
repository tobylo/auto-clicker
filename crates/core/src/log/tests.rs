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
    // Twice the cap is as far as it grows: 14 + 5 would pass 16, 14 + 2
    // reaches it.
    log.write_line("four\n");
    log.write_line("a\n");
    log.write_line("b\n");
    assert_eq!(read(&path), "one two\nthree\na\n");
    // Once the way is clear, the next line starts a new file, however short:
    // the file is still past its cap.
    fs::remove_dir_all(rotated(&path)).unwrap();
    log.write_line("5\n");
    assert_eq!(read(&path), "5\n");
    assert_eq!(read(&rotated(&path)), "one two\nthree\na\n");
}

#[test]
fn a_full_file_moved_without_a_new_one_starts_one_at_the_next_line() {
    let dir = TempDir::new();
    let path = dir.path().join("auto-clicker.log");
    let mut log = LogFile::open(path.clone(), 8).unwrap();
    log.write_line("one two\n");
    // The state a rename leaves when the new file then cannot be opened:
    // the open file is no longer at the path.
    let moved = dir.path().join("moved");
    fs::rename(&path, &moved).unwrap();
    log.write_line("three\n");
    assert_eq!(read(&path), "three\n");
    assert_eq!(read(&moved), "one two\n");
}

#[test]
fn the_cap_is_one_mebibyte() {
    assert_eq!(LOG_CAP, 1 << 20);
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

/// Set in the child process the next test starts: the path of its log.
const CHILD_LOG: &str = "AUTO_CLICKER_TEST_LOG";

#[test]
fn once_a_file_is_opened_lines_and_panics_go_there() {
    if let Some(path) = std::env::var_os(CHILD_LOG) {
        to_file(Path::new(&path)).unwrap();
        line(format_args!("auto-clicker: a line"));
        panic!("a panic");
    }
    let dir = TempDir::new();
    let path = dir.path().join("logs").join("auto-clicker.log");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "log::tests::once_a_file_is_opened_lines_and_panics_go_there",
            "--exact",
        ])
        .env(CHILD_LOG, &path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    let text = read(&path);
    assert!(text.starts_with("auto-clicker: a line\n"), "{text}");
    assert!(
        text.contains("auto-clicker: panicked at") && text.contains("a panic"),
        "{text}"
    );
}
