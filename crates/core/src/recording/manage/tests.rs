use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::*;
use crate::recording::tests::sample;
use crate::recording::{load, save};
use crate::test_support::{TempDir, permissions_apply};

/// A folder holding the sample recording; its file and path.
fn saved(dir: &TempDir) -> (FileName, PathBuf) {
    let path = save(dir.path(), &sample()).unwrap();
    let file = FileName::parse(path.file_name().unwrap().to_str().unwrap()).unwrap();
    (file, path)
}

#[test]
fn a_rename_rewrites_the_name_in_the_same_file_and_leaves_nothing_else() {
    let dir = TempDir::new();
    let (file, _) = saved(&dir);

    let changed = rename(dir.path(), &file, "Farm loop".to_owned()).unwrap();

    assert!(matches!(changed, Changed::Synced), "{changed:?}");
    let renamed = load(dir.path(), &file).unwrap();
    assert_eq!(renamed.name(), "Farm loop");
    assert_eq!(renamed, sample().renamed("Farm loop".to_owned()).unwrap());
    assert_eq!(dir.entries(), [file.as_str()]);
}

#[test]
fn a_name_a_recording_may_not_have_is_refused_before_the_file_is_read() {
    let dir = TempDir::new();
    let (file, path) = saved(&dir);
    let before = fs::read(&path).unwrap();
    for name in ["", "  ", "a\nb", &"x".repeat(201)] {
        let err = rename(dir.path(), &file, name.to_owned()).unwrap_err();
        assert!(matches!(err, ChangeError::Refused(_)), "{name:?}: {err:?}");
    }
    // A file that does not read is not even opened for a bad name.
    let missing = FileName::parse("2020-01-01_00-00-00.000.json").unwrap();
    let err = rename(dir.path(), &missing, String::new()).unwrap_err();
    assert!(matches!(err, ChangeError::Refused(_)), "{err:?}");
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn a_rename_of_a_file_gone_or_invalid_changes_nothing() {
    let dir = TempDir::new();
    let gone = FileName::parse("2020-01-01_00-00-00.000.json").unwrap();
    let err = rename(dir.path(), &gone, "New".to_owned()).unwrap_err();
    assert!(matches!(err, ChangeError::Missing), "{err:?}");
    let err = rename(&dir.path().join("none"), &gone, "New".to_owned()).unwrap_err();
    assert!(matches!(err, ChangeError::Missing), "{err:?}");
    assert_eq!(err.to_string(), "the file is no longer there");

    fs::write(dir.path().join(gone.as_str()), b"{\"version\":1,").unwrap();
    let err = rename(dir.path(), &gone, "New".to_owned()).unwrap_err();
    assert!(matches!(err, ChangeError::Unreadable(_)), "{err:?}");
    assert_eq!(
        fs::read(dir.path().join(gone.as_str())).unwrap(),
        b"{\"version\":1,"
    );
    assert_eq!(dir.entries(), [gone.as_str()]);
}

#[test]
fn a_rename_that_cannot_write_leaves_the_old_file_whole() {
    let dir = TempDir::new();
    let (file, path) = saved(&dir);
    let before = fs::read(&path).unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let renamed = rename(dir.path(), &file, "Farm loop".to_owned());
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    if !permissions_apply() {
        return;
    }
    let err = renamed.unwrap_err();
    assert!(matches!(err, ChangeError::Failed(_)), "{err:?}");
    let text = err.to_string();
    assert!(
        text.starts_with("the file could not be written: "),
        "{text}"
    );
    assert!(!text.contains("/proc/self/fd"), "{text}");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(dir.entries(), [file.as_str()]);
}

#[test]
fn a_delete_removes_the_file_and_a_second_finds_it_gone() {
    let dir = TempDir::new();
    let (file, _) = saved(&dir);

    let changed = delete(dir.path(), &file).unwrap();

    assert!(matches!(changed, Changed::Synced), "{changed:?}");
    assert!(dir.entries().is_empty());
    let err = delete(dir.path(), &file).unwrap_err();
    assert!(matches!(err, ChangeError::Missing), "{err:?}");
    let err = delete(&dir.path().join("none"), &file).unwrap_err();
    assert!(matches!(err, ChangeError::Missing), "{err:?}");
}

#[test]
fn a_delete_that_is_not_allowed_is_a_failure_not_a_file_gone() {
    let dir = TempDir::new();
    let (file, path) = saved(&dir);
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let deleted = delete(dir.path(), &file);
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    if !permissions_apply() {
        return;
    }
    let err = deleted.unwrap_err();
    assert!(
        matches!(&err, ChangeError::Failed(io) if io.kind() == io::ErrorKind::PermissionDenied),
        "{err:?}"
    );
    assert!(
        err.to_string()
            .starts_with("the file could not be deleted: "),
        "{err}"
    );
    assert!(path.exists());
}

#[test]
fn a_symlink_in_the_folders_place_is_refused() {
    let dir = TempDir::new();
    let (file, path) = saved(&dir);
    let elsewhere = TempDir::new();
    let link = elsewhere.path().join("recordings");
    std::os::unix::fs::symlink(dir.path(), &link).unwrap();
    for err in [
        delete(&link, &file).unwrap_err(),
        rename(&link, &file, "New".to_owned()).unwrap_err(),
    ] {
        assert!(matches!(err, ChangeError::Failed(_)), "{err:?}");
    }
    assert!(path.exists());
}

#[test]
fn a_rename_that_would_outgrow_what_a_load_reads_leaves_the_old_file_whole() {
    let dir = TempDir::new();
    let (file, path) = saved(&dir);
    let before = fs::read(&path).unwrap();
    let size = u64::try_from(before.len()).unwrap();
    // The same length of name fits exactly; one character more does not.
    let same = "x".repeat(sample().name().chars().count());
    rename_capped(dir.path(), &file, same.clone(), size).unwrap();
    let err = rename_capped(dir.path(), &file, format!("{same}y"), size).unwrap_err();
    assert!(
        err.to_string().contains("would be larger than 256 MiB"),
        "{err}"
    );
    assert_eq!(load(dir.path(), &file).unwrap().name(), same);
    assert_eq!(dir.entries(), [file.as_str()]);
}
