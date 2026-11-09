//! Helpers shared by this crate's tests.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// A fresh empty directory under the system temp directory, removed on drop.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new() -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "auto-clicker-test-{}-{:016x}",
            std::process::id(),
            fastrand::u64(..)
        ));
        fs::create_dir(&dir).unwrap();
        TempDir(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    /// The names in the directory, sorted.
    pub(crate) fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Whether file permissions bind this process. They do not for root (CI
/// containers often run as root), so tests that rely on a denied open skip
/// there. Probed by opening a file nobody may read.
pub(crate) fn permissions_apply() -> bool {
    let dir = TempDir::new();
    let probe = dir.path().join("probe");
    fs::write(&probe, b"").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o000)).unwrap();
    fs::File::open(&probe).is_err()
}
