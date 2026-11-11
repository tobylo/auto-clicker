//! Saving the config: what is written, and when a save waits for the
//! user's word.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::test_support::{TempDir, permissions_apply};

fn changed() -> Config {
    Config {
        tray_start_delay: Duration::from_secs(9),
        ..Config::default()
    }
}

/// A directory and the config path in it, loaded as the daemon loads it.
fn loaded_with(text: Option<&str>) -> (TempDir, PathBuf, ConfigFile) {
    let dir = TempDir::new();
    let path = dir.path().join("auto-clicker").join("config.toml");
    if let Some(text) = text {
        fs::create_dir(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
    }
    let file = Config::load(&path).file;
    (dir, path, file)
}

fn read(path: &PathBuf) -> String {
    fs::read_to_string(path).unwrap()
}

fn stale(result: Result<Saved, SaveError>) -> Option<Stale> {
    match result {
        Err(SaveError::Stale(stale)) => Some(stale),
        _ => None,
    }
}

#[test]
fn a_missing_file_is_written_with_its_directory_and_reads_back() {
    let (_dir, path, mut file) = loaded_with(None);
    file.save(&changed(), false).unwrap();
    assert_eq!(Config::parse(&read(&path)).ok(), Some(changed()));
}

#[test]
fn an_unchanged_file_is_replaced_and_saves_again_without_asking() {
    let (_dir, path, mut file) = loaded_with(Some("version = 1\n"));
    file.save(&changed(), false).unwrap();
    file.save(&Config::default(), false).unwrap();
    assert_eq!(Config::parse(&read(&path)).ok(), Some(Config::default()));
}

#[test]
fn a_file_changed_since_it_was_read_is_kept_until_the_user_agrees() {
    let (_dir, path, mut file) = loaded_with(Some("version = 1\n"));
    fs::write(&path, "version = 1\ntray_start_delay = 2\n").unwrap();
    assert_eq!(stale(file.save(&changed(), false)), Some(Stale::Changed));
    assert_eq!(read(&path), "version = 1\ntray_start_delay = 2\n");
    file.save(&changed(), true).unwrap();
    assert_eq!(Config::parse(&read(&path)).ok(), Some(changed()));
}

#[test]
fn a_file_created_since_none_was_read_counts_as_changed() {
    let (_dir, path, mut file) = loaded_with(None);
    fs::create_dir(path.parent().unwrap()).unwrap();
    fs::write(&path, "version = 1\n").unwrap();
    assert_eq!(stale(file.save(&changed(), false)), Some(Stale::Changed));
    assert_eq!(read(&path), "version = 1\n");
}

#[test]
fn a_file_removed_since_it_was_read_counts_as_changed() {
    let (_dir, path, mut file) = loaded_with(Some("version = 1\n"));
    fs::remove_file(&path).unwrap();
    assert_eq!(stale(file.save(&changed(), false)), Some(Stale::Changed));
    assert!(!path.exists());
}

#[test]
fn a_file_invalid_at_start_is_kept_until_the_user_agrees() {
    let (_dir, path, mut file) = loaded_with(Some("version = 2\n"));
    assert_eq!(stale(file.save(&changed(), false)), Some(Stale::Unusable));
    assert_eq!(read(&path), "version = 2\n");
    file.save(&changed(), true).unwrap();
    assert_eq!(Config::parse(&read(&path)).ok(), Some(changed()));
}

#[test]
fn a_file_that_cannot_be_read_now_is_kept_until_the_user_agrees() {
    let (_dir, path, mut file) = loaded_with(Some("version = 1\n"));
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(stale(file.save(&changed(), false)), Some(Stale::Unusable));
    assert!(path.is_dir());
}

#[test]
fn without_a_config_directory_nothing_is_saved() {
    let result = ConfigFile::none().save(&changed(), true);
    assert!(matches!(result, Err(SaveError::NoPath)));
}

/// Runs `body` with `dir`'s mode set to `mode`, and restores it; `None`
/// when the mode does not apply (root ignores it).
fn with_mode<T>(dir: &Path, mode: u32, body: impl FnOnce() -> T) -> Option<T> {
    let set = |mode| fs::set_permissions(dir, fs::Permissions::from_mode(mode)).unwrap();
    set(mode);
    let result = body();
    set(0o700);
    permissions_apply().then_some(result)
}

#[test]
fn a_failed_write_leaves_the_file_and_what_was_seen() {
    let (_dir, path, mut file) = loaded_with(Some("version = 1\n"));
    // No temporary file can be made in a read-only directory.
    let result = with_mode(path.parent().unwrap(), 0o500, || {
        file.save(&changed(), false)
    });
    let Some(result) = result else { return };
    assert!(matches!(result, Err(SaveError::Write(_))), "{result:?}");
    assert_eq!(read(&path), "version = 1\n");
    file.save(&changed(), false).unwrap();
}

#[test]
fn a_file_in_place_but_not_flushed_counts_as_saved_and_says_so() {
    let (_dir, path, mut file) = loaded_with(Some("version = 1\n"));
    // The directory cannot be opened to flush it, but can be written.
    let result = with_mode(path.parent().unwrap(), 0o300, || {
        file.save(&changed(), false)
    });
    let Some(result) = result else { return };
    assert!(matches!(result, Ok(Saved::Unsynced(_))), "{result:?}");
    assert_eq!(Config::parse(&read(&path)).ok(), Some(changed()));
    // What was written is what the next save compares with.
    assert!(matches!(
        file.save(&Config::default(), false),
        Ok(Saved::Synced)
    ));
}
