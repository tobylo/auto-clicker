//! What Windows itself does with a held folder, a junction and a file
//! another program holds: these run only on Windows (CI, and the test VM).

use std::fs;
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use super::super::{Folder, Link, open_read};
use crate::test_support::TempDir;

/// Makes `link` a junction to the folder `target`, as `mklink /J` does; no
/// privilege is needed for one.
fn junction(link: &Path, target: &Path) {
    let made = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
}

/// `path` opened so that no other program may read, write or remove it
/// until the handle is dropped, as a scanner that takes a file whole does.
fn hold(path: &Path) -> fs::File {
    fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_junction_in_place_of_the_folder_is_refused_and_nothing_is_written() {
    let root = TempDir::new();
    let elsewhere = root.path().join("project");
    fs::create_dir(&elsewhere).unwrap();
    let link = root.path().join("recordings");
    junction(&link, &elsewhere);

    let opened = Folder::open(&link).err().unwrap();
    let private = Folder::open_private(&link).err().unwrap();

    assert_eq!(opened.kind(), io::ErrorKind::NotADirectory, "{opened}");
    assert_eq!(private.kind(), io::ErrorKind::NotADirectory, "{private}");
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[test]
fn a_held_folder_and_its_ancestors_cannot_be_moved_or_deleted() {
    let root = TempDir::new();
    let parent = root.path().join("data").join("auto-clicker");
    let dir = parent.join("recordings");
    fs::create_dir_all(&dir).unwrap();
    let folder = Folder::open(&dir).unwrap();

    let moved = fs::rename(&dir, root.path().join("moved"));
    let parent_moved = fs::rename(&parent, root.path().join("parent-moved"));
    let grandparent_moved = fs::rename(root.path().join("data"), root.path().join("data-moved"));
    let deleted = fs::remove_dir(&dir);

    assert!(moved.is_err(), "the held folder was moved");
    assert!(parent_moved.is_err(), "the held folder's parent was moved");
    assert!(
        grandparent_moved.is_err(),
        "the held folder's grandparent was moved"
    );
    assert!(deleted.is_err(), "the held folder was deleted");
    let written = folder
        .create_new(&names(&["a.json"]), |out| out.write_all(b"kept"))
        .unwrap();
    assert_eq!(fs::read(dir.join(written)).unwrap(), b"kept");
}

#[test]
fn a_junction_that_leads_nowhere_is_refused_not_missing() {
    let root = TempDir::new();
    let gone = root.path().join("gone");
    fs::create_dir(&gone).unwrap();
    let link = root.path().join("recordings");
    junction(&link, &gone);
    fs::remove_dir(&gone).unwrap();

    let err = Folder::open(&link).err().unwrap();

    assert_eq!(err.kind(), io::ErrorKind::NotADirectory, "{err}");
}

#[test]
fn a_junction_on_the_way_swapped_after_the_check_never_receives_the_file() {
    let root = TempDir::new();
    let real = root.path().join("real");
    fs::create_dir_all(real.join("recordings")).unwrap();
    let elsewhere = root.path().join("elsewhere");
    fs::create_dir_all(elsewhere.join("recordings")).unwrap();
    // The data folder is a junction, as a moved profile folder may be.
    let data = root.path().join("data");
    junction(&data, &real);
    let folder = Folder::open(&data.join("recordings")).unwrap();
    // A junction itself is not held: it can be removed and pointed elsewhere.
    fs::remove_dir(&data).unwrap();
    junction(&data, &elsewhere);

    let written = folder
        .create_new(&names(&["a.json"]), |out| out.write_all(b"kept"))
        .unwrap();

    assert_eq!(
        fs::read(real.join("recordings").join(written)).unwrap(),
        b"kept"
    );
    assert_eq!(
        fs::read_dir(elsewhere.join("recordings")).unwrap().count(),
        0
    );
}

#[test]
fn an_error_names_the_folder_as_it_was_given() {
    let root = TempDir::new();
    let folder = Folder::open(root.path()).unwrap();
    let held = folder.path().join("x.json");
    let err = io::Error::other(format!("{} is taken", held.display()));

    let shown = folder.named(root.path(), &err).to_string();

    let expected = root.path().join("x.json");
    assert_eq!(shown, format!("{} is taken", expected.display()));
}

#[test]
fn a_junction_in_place_of_a_file_is_refused_unless_followed() {
    let root = TempDir::new();
    let target = root.path().join("target");
    fs::create_dir(&target).unwrap();
    let link = root.path().join("a.json");
    junction(&link, &target);
    let folder = Folder::open(root.path()).unwrap();

    let in_folder = folder.open_file("a.json").unwrap_err();
    let refused = open_read(&link, Link::Refuse).unwrap_err();

    for err in [in_folder, refused] {
        assert!(
            err.to_string()
                .ends_with("is a link, which is not followed"),
            "{err}"
        );
    }
}

#[test]
fn a_file_another_program_holds_is_busy_not_missing() {
    let root = TempDir::new();
    let path = root.path().join("config.toml");
    fs::write(&path, b"old").unwrap();
    let held = hold(&path);
    let started = Instant::now();

    let read = open_read(&path, Link::Follow).unwrap_err();
    let replaced = super::super::atomic_write(&path, b"new").unwrap_err();

    // Each waited about 300 ms; well under the bound however slow the runner.
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(read.kind(), io::ErrorKind::ResourceBusy, "{read}");
    assert!(
        matches!(
            replaced.kind(),
            io::ErrorKind::ResourceBusy | io::ErrorKind::PermissionDenied
        ),
        "{replaced}"
    );
    drop(held);
    assert_eq!(fs::read(&path).unwrap(), b"old");
    assert_eq!(root.entries(), ["config.toml"]);
}

#[test]
fn a_file_let_go_within_the_wait_is_replaced() {
    let root = TempDir::new();
    let path = root.path().join("config.toml");
    fs::write(&path, b"old").unwrap();
    let held = hold(&path);
    let release = thread::spawn(move || {
        thread::sleep(Duration::from_millis(40));
        drop(held);
    });

    let replaced = super::super::atomic_write(&path, b"new");

    release.join().unwrap();
    replaced.unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"new");
    assert_eq!(root.entries(), ["config.toml"]);
}

#[test]
fn a_file_let_go_within_the_wait_is_read() {
    let root = TempDir::new();
    let path = root.path().join("config.toml");
    fs::write(&path, b"text").unwrap();
    let held = hold(&path);
    let release = thread::spawn(move || {
        thread::sleep(Duration::from_millis(40));
        drop(held);
    });

    let read = open_read(&path, Link::Follow).map(io::read_to_string);

    release.join().unwrap();
    assert_eq!(read.unwrap().unwrap(), "text");
}
