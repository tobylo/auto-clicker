use super::*;
use crate::test_support::TempDir;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_new_file_is_placed_only_where_none_is() {
    let dir = TempDir::new();
    let path = dir.path().join("config.toml");
    atomic_save(&path, b"first", Place::New).unwrap().unwrap();
    let err = atomic_save(&path, b"second", Place::New).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&path).unwrap(), b"first");
    assert_eq!(dir.entries(), ["config.toml"]);
}

#[test]
fn writes_a_new_file_and_leaves_nothing_else() {
    let dir = TempDir::new();
    let path = dir.path().join("config.toml");

    atomic_write(&path, b"version = 1\n").unwrap();

    assert_eq!(fs::read(&path).unwrap(), b"version = 1\n");
    assert_eq!(dir.entries(), ["config.toml"]);
}

#[test]
fn replaces_an_existing_file_whole() {
    let dir = TempDir::new();
    let path = dir.path().join("config.toml");
    fs::write(&path, b"a much longer old content than the new one").unwrap();

    atomic_write(&path, b"new").unwrap();

    assert_eq!(fs::read(&path).unwrap(), b"new");
    assert_eq!(dir.entries(), ["config.toml"]);
}

#[test]
fn a_failed_rename_removes_the_temp_file_and_leaves_the_target() {
    let dir = TempDir::new();
    // A non-empty directory cannot be replaced by a file.
    let path = dir.path().join("taken");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("inside"), b"kept").unwrap();

    assert!(atomic_write(&path, b"new").is_err());

    assert_eq!(dir.entries(), ["taken"]);
    assert_eq!(fs::read(path.join("inside")).unwrap(), b"kept");
}

#[test]
fn a_failure_at_any_step_before_the_placing_leaves_the_old_file_whole() {
    let dir = TempDir::new();
    let path = dir.path().join("recording.json");
    fs::write(&path, b"old").unwrap();
    let patience = Patience::new();
    let beside = Beside {
        dir: dir.path(),
        path: &path,
        patience: &patience,
    };
    let unplaced =
        |_: &mut PathBuf| -> io::Result<PathBuf> { panic!("placed after a failed write") };
    let crashed_writing = write_then_place(
        beside,
        |out| {
            out.write_all(b"half")?;
            Err(io::Error::other("crashed mid-write"))
        },
        unplaced,
        || Ok(()),
    );
    let crashed_placing = write_then_place(
        beside,
        |out| out.write_all(b"new"),
        |_| -> io::Result<PathBuf> { Err(io::Error::other("crashed placing")) },
        || Ok(()),
    );
    // The temporary file renamed after another name first: that one goes.
    let crashed_renamed = write_then_place(
        beside,
        |out| out.write_all(b"new"),
        |temp| -> io::Result<PathBuf> {
            let next = temp.with_file_name(".recording-1.json.1.0123456789abcdef.tmp");
            fs::rename(&*temp, &next)?;
            *temp = next;
            Err(io::Error::other("crashed after a rename"))
        },
        || Ok(()),
    );

    let errors =
        [crashed_writing, crashed_placing, crashed_renamed].map(|err| err.unwrap_err().to_string());
    assert_eq!(
        errors,
        [
            "crashed mid-write",
            "crashed placing",
            "crashed after a rename"
        ]
    );
    assert_eq!(fs::read(&path).unwrap(), b"old");
    assert_eq!(dir.entries(), ["recording.json"]);
}

#[test]
fn a_crash_after_writing_leaves_the_old_file_whole_and_a_temp_the_next_start_removes() {
    let dir = TempDir::new();
    let path = dir.path().join("recording.json");
    fs::write(&path, b"old").unwrap();
    // The process dies after the temporary file is flushed, before it is
    // placed: nothing removes it.
    let (temp, file) = create_temp(Beside {
        dir: dir.path(),
        path: &path,
        patience: &Patience::new(),
    })
    .unwrap();
    fill(file, |out| out.write_all(b"new")).unwrap();
    assert!(temp.exists());

    let folder = Folder::open(dir.path()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"old");
    assert_eq!(folder.remove_temps(Duration::from_secs(3600)).unwrap(), 0);
    assert_eq!(folder.remove_temps(Duration::ZERO).unwrap(), 1);
    assert_eq!(dir.entries(), ["recording.json"]);
}

#[test]
fn a_missing_directory_is_an_error() {
    let dir = TempDir::new();
    let path = dir.path().join("missing").join("config.toml");

    let err = atomic_write(&path, b"x").unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    assert_eq!(dir.entries(), Vec::<String>::new());
}

#[test]
fn a_bare_file_name_is_in_the_current_directory() {
    let dirs = (
        parent_dir(Path::new("config.toml")).unwrap(),
        parent_dir(Path::new("/etc/config.toml")).unwrap(),
    );
    assert_eq!(dirs, (Path::new("."), Path::new("/etc")));
}

#[test]
fn a_path_without_a_file_name_is_an_error() {
    let dir = TempDir::new();
    for path in [Path::new("/").to_owned(), dir.path().join("..")] {
        let err = atomic_write(&path, b"x").unwrap_err();
        assert_eq!(
            err.kind(),
            io::ErrorKind::InvalidInput,
            "{}",
            path.display()
        );
    }
}

#[test]
fn create_writes_the_first_free_name_and_never_replaces() {
    let dir = TempDir::new();
    let tried = names(&["a.json", "a-1.json", "a-2.json"]);
    fs::write(dir.path().join("a.json"), b"kept").unwrap();
    let folder = Folder::open(dir.path()).unwrap();

    let first = folder
        .create_new(&tried, |out| out.write_all(b"one"))
        .unwrap();
    let second = folder
        .create_new(&tried, |out| out.write_all(b"two"))
        .unwrap();
    let full = folder
        .create_new(&tried, |out| out.write_all(b"three"))
        .unwrap_err();

    assert_eq!((first.as_str(), second.as_str()), ("a-1.json", "a-2.json"));
    assert_eq!(full.kind(), io::ErrorKind::AlreadyExists);
    let named = folder.named(dir.path(), &full).to_string();
    let first_path = dir.path().join("a.json").display().to_string();
    assert!(named.starts_with(&first_path), "{named}");
    let contents: Vec<Vec<u8>> = tried
        .iter()
        .map(|name| fs::read(dir.path().join(name)).unwrap())
        .collect();
    assert_eq!(
        contents,
        [b"kept".to_vec(), b"one".to_vec(), b"two".to_vec()]
    );
    assert_eq!(dir.entries(), ["a-1.json", "a-2.json", "a.json"]);
}

#[test]
fn a_create_that_fails_for_another_reason_than_a_taken_name_says_so() {
    let dir = TempDir::new();
    fs::write(dir.path().join("a.json"), b"kept").unwrap();
    // Longer than any file system allows one name to be.
    let too_long = format!("{}.json", "a".repeat(300));
    let folder = Folder::open(dir.path()).unwrap();

    let err = folder
        .create_new(&names(&["a.json", &too_long, "a-2.json"]), |out| {
            out.write_all(b"new")
        })
        .unwrap_err();

    assert_ne!(err.kind(), io::ErrorKind::AlreadyExists, "{err}");
    assert_eq!(dir.entries(), ["a.json"]);
}

#[test]
fn a_start_keeps_a_temp_file_younger_than_an_hour() {
    let dir = TempDir::new();
    let young = dir.path().join(".a.json.1.0123456789abcdef.tmp");
    let old = dir.path().join(".a.json.2.0123456789abcdef.tmp");
    let now = std::time::SystemTime::now();
    for (path, ago) in [(&young, 59 * 60), (&old, 61 * 60)] {
        fs::write(path, b"half").unwrap();
        let at = now - Duration::from_secs(ago);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    let removed = remove_stale_temps(&dir.path().join("a.json"), STALE_TEMP_AGE).unwrap();

    assert_eq!(removed, 1);
    assert_eq!(dir.entries(), [".a.json.1.0123456789abcdef.tmp"]);
}

#[test]
fn a_temp_file_is_renamed_after_each_name_before_it_is_tried_there() {
    let dir = TempDir::new();
    // Taken: a file, and a folder a placing cannot replace either.
    fs::write(dir.path().join("a.json"), b"kept").unwrap();
    fs::create_dir(dir.path().join("a-1.json")).unwrap();
    let folder = Folder::open(dir.path()).unwrap();
    let (mut temp, file) = create_temp(Beside {
        dir: folder.path(),
        path: &folder.path().join("a.json"),
        patience: &Patience::new(),
    })
    .unwrap();
    fill(file, |out| out.write_all(b"new")).unwrap();

    let err = folder
        .place_at_first_free(&mut temp, &names(&["a.json", "a-1.json"]))
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    let name = temp.file_name().unwrap().to_str().unwrap();
    assert_eq!(temps::target_of(name), Some("a-1.json"), "{name}");
    assert_eq!(fs::read(&temp).unwrap(), b"new");
    assert_eq!(dir.entries().len(), 3);
}

#[test]
fn placing_under_a_name_no_file_system_takes_is_an_error_not_a_taken_name() {
    let dir = TempDir::new();
    let folder = Folder::open(dir.path()).unwrap();
    let (mut temp, file) = create_temp(Beside {
        dir: folder.path(),
        path: &folder.path().join("a.json"),
        patience: &Patience::new(),
    })
    .unwrap();
    fill(file, |out| out.write_all(b"new")).unwrap();
    let too_long = format!("{}.json", "a".repeat(300));

    let err = folder
        .place_if_free(&mut temp, false, &too_long)
        .unwrap_err();

    assert_ne!(err.kind(), io::ErrorKind::AlreadyExists, "{err}");
    assert!(temp.exists());
}

#[test]
fn a_failed_create_leaves_no_file_behind() {
    let dir = TempDir::new();
    let folder = Folder::open(dir.path()).unwrap();

    let err = folder
        .create_new(&names(&["recording.json"]), |out| {
            out.write_all(b"half a reco")?;
            Err(io::Error::other("crashed mid-write"))
        })
        .unwrap_err();
    let none = folder.create_new(&[], |_| Ok(())).unwrap_err();

    assert_eq!(err.to_string(), "crashed mid-write");
    assert_eq!(none.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(dir.entries(), Vec::<String>::new());
}

#[test]
fn a_replace_in_a_folder_rewrites_the_file_whole() {
    let dir = TempDir::new();
    fs::write(dir.path().join("a.json"), b"a much longer old file").unwrap();
    let folder = Folder::open(dir.path()).unwrap();

    folder
        .replace("a.json", |out| out.write_all(b"new"))
        .unwrap()
        .unwrap();

    assert_eq!(fs::read(dir.path().join("a.json")).unwrap(), b"new");
    assert_eq!(dir.entries(), ["a.json"]);
}

#[test]
fn a_folder_reads_and_removes_its_files_by_name() {
    let dir = TempDir::new();
    fs::write(dir.path().join("a.json"), b"text").unwrap();
    let folder = Folder::open(dir.path()).unwrap();

    let read = io::read_to_string(folder.open_file("a.json").unwrap()).unwrap();
    let listed = folder.entries().unwrap().count();
    folder.remove_file("a.json").unwrap();
    let gone = folder.open_file("a.json").unwrap_err();

    assert_eq!((read.as_str(), listed), ("text", 1));
    assert_eq!(gone.kind(), io::ErrorKind::NotFound);
    assert_eq!(dir.entries(), Vec::<String>::new());
}

#[test]
fn a_name_in_a_folder_is_one_plain_component() {
    let dir = TempDir::new();
    let folder = Folder::open(dir.path()).unwrap();
    let outside = ["../x.json", "a/b.json", "", ".", "..", "/x.json", "a/"];
    for name in outside {
        let kinds = [
            folder.open_file(name).err().map(|err| err.kind()),
            folder.remove_file(name).err().map(|err| err.kind()),
            folder.replace(name, |_| Ok(())).err().map(|err| err.kind()),
            folder
                .create_new(&names(&[name]), |_| Ok(()))
                .err()
                .map(|err| err.kind()),
        ];
        assert_eq!(kinds, [Some(io::ErrorKind::InvalidInput); 4], "{name:?}");
    }
    assert_eq!(dir.entries(), Vec::<String>::new());
}

#[test]
fn a_private_folder_must_be_absolute_and_is_made_whole() {
    let dir = TempDir::new();
    let nested = dir.path().join("new/data/auto-clicker/recordings");
    let relative = Folder::open_private(Path::new("recordings"))
        .err()
        .map(|err| err.kind());

    let folder = Folder::open_private(&nested).unwrap();

    assert_eq!(relative, Some(io::ErrorKind::InvalidInput));
    assert!(nested.is_dir());
    folder.sync().unwrap();
}

#[test]
fn a_plain_file_is_read_whether_links_are_followed_or_not() {
    let dir = TempDir::new();
    let path = dir.path().join("config.toml");
    fs::write(&path, b"text").unwrap();
    for link in [Link::Follow, Link::Refuse] {
        let read = io::read_to_string(open_read(&path, link).unwrap()).unwrap();
        assert_eq!(read, "text");
    }
    let missing = open_read(&dir.path().join("missing"), Link::Follow).unwrap_err();
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::test_support::permissions_apply;

    #[test]
    fn a_file_placed_in_a_directory_that_cannot_be_flushed_is_in_place() {
        let dir = TempDir::new();
        let path = dir.path().join("config.toml");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o300)).unwrap();
        let saved = atomic_save(&path, b"new", Place::Replace);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        if !permissions_apply() {
            return;
        }
        let unsynced = saved.unwrap().unwrap_err().into_error();
        assert!(unsynced.to_string().contains("was written"), "{unsynced}");
        assert_eq!(fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn the_new_file_is_private_to_its_owner() {
        let dir = TempDir::new();
        let path = dir.path().join("recording.json");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let folder = Folder::open(dir.path()).unwrap();

        atomic_write(&path, b"new").unwrap();
        let tried = names(&["new.json"]);
        let created = folder
            .create_new(&tried, |out| out.write_all(b"one"))
            .unwrap();

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            (mode(&path), mode(&dir.path().join(created))),
            (0o600, 0o600)
        );
    }

    #[test]
    fn a_failed_directory_sync_says_the_file_was_replaced() {
        if !permissions_apply() {
            return;
        }
        let dir = TempDir::new();
        // Writable and searchable but not readable: the rename works, and
        // opening the directory to sync it fails.
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o300)).unwrap();
        let path = locked.join("config.toml");

        let result = atomic_write(&path, b"new");

        // Restored before any assertion, so the directory can be removed.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        let message = result.err().map(|err| err.to_string()).unwrap_or_default();
        let replaced = message.contains("was written, but syncing its directory");
        let contents = fs::read(&path).unwrap();
        assert_eq!((replaced, contents), (true, b"new".to_vec()), "{message}");
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
        let (send, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || send.send(sync_dir(&fifo)));
        let synced = received.recv_timeout(Duration::from_secs(5));
        let err = synced.expect("the flush waited on the FIFO").unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::ENOTDIR), "{err}");
    }

    #[test]
    fn a_symlinked_folder_or_file_is_refused() {
        let root = TempDir::new();
        let real = root.path().join("real");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("a.json"), b"").unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        std::os::unix::fs::symlink(real.join("a.json"), real.join("b.json")).unwrap();

        let folder = Folder::open(&link).err().unwrap();
        let file = Folder::open(&real)
            .unwrap()
            .open_file("b.json")
            .unwrap_err();
        let read = open_read(&real.join("b.json"), Link::Refuse).unwrap_err();

        // O_NOFOLLOW with O_DIRECTORY: a symlink is "not a directory".
        assert_eq!(folder.raw_os_error(), Some(libc::ENOTDIR), "{folder}");
        assert_eq!(file.raw_os_error(), Some(libc::ELOOP), "{file}");
        assert_eq!(read.raw_os_error(), Some(libc::ELOOP), "{read}");
        assert!(open_read(&real.join("b.json"), Link::Follow).is_ok());
    }

    #[test]
    fn a_folder_others_can_read_is_made_private() {
        let root = TempDir::new();
        let dir = root.path().join("recordings");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        Folder::open_private(&dir).unwrap();

        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn an_ancestor_that_may_be_searched_but_not_read_ends_the_flush() {
        if !permissions_apply() {
            return;
        }
        let root = TempDir::new();
        let closed = root.path().join("closed");
        let dir = closed.join("alice/data/auto-clicker/recordings");
        fs::create_dir_all(closed.join("alice")).unwrap();
        fs::set_permissions(&closed, fs::Permissions::from_mode(0o311)).unwrap();

        let opened = Folder::open_private(&dir);

        fs::set_permissions(&closed, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(opened.is_ok(), "{:?}", opened.err());
    }

    #[test]
    fn an_error_names_the_checked_folder_where_it_is_now() {
        let root = TempDir::new();
        let dir = root.path().join("recordings");
        fs::create_dir(&dir).unwrap();
        let folder = Folder::open(&dir).unwrap();
        let moved = root.path().join("moved");
        fs::rename(&dir, &moved).unwrap();
        let held = folder.path().join("x.json");
        let err = io::Error::other(format!("{} is taken", held.display()));

        let shown = folder.named(&dir, &err).to_string();

        let expected = fs::canonicalize(&moved).unwrap().join("x.json");
        assert_eq!(shown, format!("{} is taken", expected.display()));
    }
}
