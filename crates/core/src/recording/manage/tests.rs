use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::*;
use crate::recording::tests::sample;
use crate::recording::{load, save};
use crate::test_support::TempDir;
#[cfg(unix)]
use crate::test_support::permissions_apply;

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

#[cfg(unix)]
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
    assert!(
        matches!(
            err,
            ChangeError::NotDone {
                step: Step::Written,
                ..
            }
        ),
        "{err:?}"
    );
    let text = err.to_string();
    assert!(
        text.starts_with("the file could not be written: "),
        "{text}"
    );
    assert!(!text.contains("/proc/self/fd"), "{text}");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(dir.entries(), [file.as_str()]);
}

/// A folder holding the sample recording `n` times, in files of their own.
fn saved_many(dir: &TempDir, n: usize) -> Vec<FileName> {
    (0..n).map(|_| saved(dir).0).collect()
}

#[test]
fn a_delete_removes_the_files_named_and_counts_one_gone_as_not_deleted() {
    let dir = TempDir::new();
    let files = saved_many(&dir, 3);

    let removed = delete(dir.path(), Doomed::These(&files[..2]), 8).unwrap();

    let whole = removed.failed.is_empty() && removed.more_failed == 0;
    assert!(whole && removed.unsynced.is_none(), "{removed:?}");
    assert_eq!(
        (removed.deleted, dir.entries()),
        (2, vec![files[2].to_string()])
    );
    let again = delete(dir.path(), Doomed::These(&files[..1]), 8).unwrap();
    let [(file, err)] = &again.failed[..] else {
        panic!("{again:?}");
    };
    assert_eq!((again.deleted, file), (0, &files[0]));
    assert!(matches!(err, ChangeError::Missing), "{err:?}");
    let err = delete(&dir.path().join("none"), Doomed::All, 8).unwrap_err();
    assert!(matches!(err, ChangeError::Missing), "{err:?}");
}

#[test]
fn deleting_all_takes_every_recording_and_nothing_else() {
    let dir = TempDir::new();
    saved_many(&dir, 3);
    fs::write(dir.path().join("notes.txt"), b"keep").unwrap();

    let removed = delete(dir.path(), Doomed::All, 8).unwrap();

    assert_eq!(removed.deleted, 3);
    assert!(removed.failed.is_empty(), "{removed:?}");
    assert_eq!(dir.entries(), ["notes.txt"]);
    let empty = delete(dir.path(), Doomed::All, 8).unwrap();
    assert_eq!(empty.deleted, 0);
    assert!(empty.failed.is_empty(), "{empty:?}");
}

/// The name of a temporary file a save of `target` left.
fn temp_of(target: &str, pid: u32) -> String {
    format!(".{target}.{pid}.0123456789abcdef.tmp")
}

#[test]
fn a_delete_also_removes_what_saves_of_its_recordings_left_and_nothing_else() {
    let dir = TempDir::new();
    let files = saved_many(&dir, 2);
    let doomed = temp_of(files[0].as_str(), 7);
    // Another session saving a new recording, and a copy of the one kept.
    let saving = temp_of("2030-01-01_00-00-00.000.json", 8);
    let kept = temp_of(files[1].as_str(), 9);
    for name in [&doomed, &saving, &kept, &".notes.tmp".to_owned()] {
        fs::write(dir.path().join(name), b"a copy").unwrap();
    }

    let removed = delete(dir.path(), Doomed::These(&files[..1]), 8).unwrap();

    assert_eq!((removed.deleted, removed.failed.len()), (1, 0));
    let mut left = vec![".notes.tmp".to_owned(), files[1].to_string(), saving, kept];
    left.sort();
    assert_eq!(dir.entries(), left);
}

#[test]
fn a_delete_of_all_keeps_what_a_save_still_running_writes() {
    let dir = TempDir::new();
    let files = saved_many(&dir, 2);
    let saving = temp_of("2030-01-01_00-00-00.000.json", 8);
    for name in [
        temp_of(files[0].as_str(), 7),
        temp_of(files[1].as_str(), 7),
        saving.clone(),
    ] {
        fs::write(dir.path().join(name), b"a copy").unwrap();
    }

    let removed = delete(dir.path(), Doomed::All, 8).unwrap();

    assert_eq!(removed.deleted, 2);
    assert_eq!(dir.entries(), [saving]);
}

#[cfg(windows)]
#[test]
fn a_hidden_copy_another_program_holds_fails_its_recordings_deletion() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = TempDir::new();
    let files = saved_many(&dir, 2);
    let copy = dir.path().join(temp_of(files[0].as_str(), 7));
    fs::write(&copy, b"a copy").unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&copy)
        .unwrap();

    let removed = delete(dir.path(), Doomed::These(&files), 8).unwrap();

    drop(held);
    assert_eq!(removed.deleted, 1);
    let (file, err) = &removed.failed[0];
    assert_eq!(file, &files[0]);
    let text = format!("{err:?}");
    assert!(
        text.contains("a hidden copy of it could not be removed"),
        "{text}"
    );
    // Kept whole, so deleting it again finishes the job.
    assert!(copy.exists() && dir.path().join(files[0].as_str()).exists());
    let again = delete(dir.path(), Doomed::All, 8).unwrap();
    assert_eq!((again.deleted, again.failed.len()), (1, 0));
    assert_eq!(dir.entries(), Vec::<String>::new());
}

#[test]
fn a_delete_removes_the_copies_saves_and_renames_under_a_second_name_left() {
    let dir = TempDir::new();
    let files = saved_many(&dir, 2);
    // The second save of the same moment took `-1`; its temporary file is
    // named after the plain name it tried first.
    assert!(files[1].as_str().ends_with("-1.json"), "{}", files[1]);
    // And one a rename of the `-1` file left, named after it.
    for copy in [temp_of(files[0].as_str(), 7), temp_of(files[1].as_str(), 7)] {
        fs::write(dir.path().join(&copy), b"a copy").unwrap();
    }

    let removed = delete(dir.path(), Doomed::These(&files[1..]), 8).unwrap();

    assert_eq!(removed.deleted, 1);
    assert_eq!(dir.entries(), [files[0].to_string()]);
}

#[test]
fn a_file_that_cannot_be_removed_leaves_the_rest_going_and_the_first_are_named() {
    let dir = TempDir::new();
    let files = saved_many(&dir, 2);
    // A folder with a recording's name: a file removal cannot remove it.
    let stuck: Vec<FileName> = [
        "2020-01-01_00-00-00.000.json",
        "2020-01-01_00-00-01.000.json",
    ]
    .iter()
    .map(|name| {
        fs::create_dir(dir.path().join(name)).unwrap();
        FileName::parse(name).unwrap()
    })
    .collect();
    let doomed = [
        stuck[0].clone(),
        files[0].clone(),
        stuck[1].clone(),
        files[1].clone(),
    ];

    let removed = delete(dir.path(), Doomed::These(&doomed), 1).unwrap();

    assert_eq!(removed.deleted, 2);
    let [(file, err)] = &removed.failed[..] else {
        panic!("{removed:?}");
    };
    assert_eq!(file, &stuck[0]);
    assert!(
        matches!(
            err,
            ChangeError::NotDone {
                step: Step::Deleted,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(removed.more_failed, 1);
    let mut left = dir.entries();
    left.sort();
    assert_eq!(left, [stuck[0].as_str(), stuck[1].as_str()]);
    // Deleting all names the same ones.
    let all = delete(dir.path(), Doomed::All, 8).unwrap();
    assert_eq!((all.deleted, all.failed.len()), (0, 2));
}

#[cfg(unix)]
#[test]
fn a_delete_that_is_not_allowed_is_a_failure_not_a_file_gone() {
    let dir = TempDir::new();
    let (file, path) = saved(&dir);
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let deleted = delete(dir.path(), Doomed::These(std::slice::from_ref(&file)), 8);
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    if !permissions_apply() {
        return;
    }
    let removed = deleted.unwrap();
    let [(_, err)] = &removed.failed[..] else {
        panic!("{removed:?}");
    };
    assert!(
        matches!(err, ChangeError::NotDone { step: Step::Deleted, err } if err.kind() == io::ErrorKind::PermissionDenied),
        "{err:?}"
    );
    let said = err.to_string();
    assert!(
        said.starts_with("the file could not be deleted: "),
        "{said}"
    );
    assert_eq!((path.exists(), removed.deleted), (true, 0));
}

#[cfg(unix)]
#[test]
fn a_folder_that_cannot_be_opened_deletes_nothing() {
    let dir = TempDir::new();
    let (_, path) = saved(&dir);
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o300)).unwrap();
    let deleted = delete(dir.path(), Doomed::All, 8);
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    if !permissions_apply() {
        return;
    }
    let err = deleted.unwrap_err();
    assert!(matches!(err, ChangeError::Failed(_)), "{err:?}");
    assert!(path.exists());
}

#[cfg(unix)]
#[test]
fn a_symlink_in_the_folders_place_is_refused() {
    let dir = TempDir::new();
    let (file, path) = saved(&dir);
    let elsewhere = TempDir::new();
    let link = elsewhere.path().join("recordings");
    std::os::unix::fs::symlink(dir.path(), &link).unwrap();
    for err in [
        delete(&link, Doomed::These(std::slice::from_ref(&file)), 8).unwrap_err(),
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
    assert!(matches!(err, ChangeError::TooLarge), "{err:?}");
    assert_eq!(
        err.to_string(),
        "the file could not be written: the renamed recording would be larger than 256 MiB"
    );
    assert_eq!(load(dir.path(), &file).unwrap().name(), same);
    assert_eq!(dir.entries(), [file.as_str()]);
}

#[test]
fn only_the_caps_own_error_is_too_large() {
    let bare = || io::Error::from(io::ErrorKind::FileTooLarge);
    assert!(too_large(true, &bare()));
    assert!(!too_large(false, &bare()));
    let left = io::Error::new(
        io::ErrorKind::FileTooLarge,
        "file too large; removing .tmp failed too: denied",
    );
    assert!(!too_large(true, &left));
    assert!(!too_large(
        true,
        &io::Error::from(io::ErrorKind::StorageFull)
    ));
}
