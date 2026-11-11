//! Where the scripts are written for KWin to load: a directory of the
//! daemon's own under `$XDG_RUNTIME_DIR` (brief decision 4).
//!
//! The directory is opened once, without following a symlink, and checked
//! on that handle: owned by this user and closed to everyone else. Every
//! file is then created through the same handle, exclusively, with mode
//! 0600, under a name no earlier load used. A directory that fails the check
//! is not used, and nothing is loaded.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, CWD, Dir, Mode, OFlags, fstat, mkdirat, openat, unlinkat};
use rustix::process::geteuid;

use crate::script::Script;

/// The directory's name under `$XDG_RUNTIME_DIR`.
const NAME: &str = "auto-clicker";

/// The checked directory.
#[derive(Debug)]
pub(crate) struct ScriptDir {
    fd: OwnedFd,
    path: PathBuf,
}

impl ScriptDir {
    /// The directory in `$XDG_RUNTIME_DIR`.
    pub(crate) fn from_env() -> Result<ScriptDir, String> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or_else(|| "XDG_RUNTIME_DIR is not set".to_owned())?;
        ScriptDir::open(Path::new(&runtime))
    }

    /// The directory in `runtime`, made when it is missing.
    pub(crate) fn open(runtime: &Path) -> Result<ScriptDir, String> {
        let path = runtime.join(NAME);
        let failed = |what: &str, err: io::Error| format!("{what} {}: {err}", path.display());
        match mkdirat(CWD, &path, Mode::RWXU) {
            Ok(()) => {}
            Err(err) if err == rustix::io::Errno::EXIST => {}
            Err(err) => return Err(failed("cannot make", err.into())),
        }
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let fd = openat(CWD, &path, flags, Mode::empty())
            .map_err(|err| failed("cannot open", err.into()))?;
        let stat = fstat(&fd).map_err(|err| failed("cannot check", err.into()))?;
        if stat.st_uid != geteuid().as_raw() {
            return Err(format!("{} belongs to another user", path.display()));
        }
        if stat.st_mode & 0o077 != 0 {
            return Err(format!("{} is open to other users", path.display()));
        }
        Ok(ScriptDir { fd, path })
    }

    /// Writes `source` to a new file named `name`, and gives the path KWin
    /// is to load. A file that could not be written whole is removed again.
    pub(crate) fn write(&self, name: &str, source: &str) -> Result<PathBuf, String> {
        let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW;
        let fd = openat(
            &self.fd,
            name,
            flags | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|err| format!("cannot create the script {name}: {err}"))?;
        if let Err(err) = File::from(fd).write_all(source.as_bytes()) {
            self.remove(name);
            return Err(format!("cannot write the script {name}: {err}"));
        }
        Ok(self.path.join(name))
    }

    /// Removes every file of `script`'s loads; every script's when `None`.
    /// Anything else in the directory is left alone.
    pub(crate) fn sweep(&self, script: Option<Script>) {
        let names = match self.names() {
            Ok(names) => names,
            Err(err) => {
                return eprintln!("auto-clicker: cannot list {}: {err}", self.path.display());
            }
        };
        let wanted = |name: &str| match script {
            Some(script) => is_file_of(name, script),
            None => Script::ALL.iter().any(|&script| is_file_of(name, script)),
        };
        names
            .iter()
            .filter(|name| wanted(name))
            .for_each(|name| self.remove(name));
    }

    /// The names in the directory, read through its handle.
    fn names(&self) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in Dir::read_from(&self.fd)? {
            let entry = entry?;
            let name = OsStr::from_bytes(entry.file_name().to_bytes()).to_string_lossy();
            names.push(name.into_owned());
        }
        Ok(names)
    }

    /// Removes `name`; a symlink is removed, not followed.
    fn remove(&self, name: &str) {
        match unlinkat(&self.fd, name, AtFlags::empty()) {
            Ok(()) => {}
            Err(err) if err == rustix::io::Errno::NOENT => {}
            Err(err) => eprintln!("auto-clicker: cannot remove the script {name}: {err}"),
        }
    }
}

/// Whether `name` is a file of a load of `script`.
fn is_file_of(name: &str, script: Script) -> bool {
    name.strip_prefix(script.plugin())
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|rest| rest.ends_with(".js"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;
    use crate::token::Token;

    /// A fresh directory standing in for `$XDG_RUNTIME_DIR`.
    struct Runtime(PathBuf);

    impl Runtime {
        fn new() -> Runtime {
            let path = std::env::temp_dir().join(format!(
                "auto-clicker-kwin-{}-{}",
                std::process::id(),
                fastrand::u64(..)
            ));
            fs::create_dir(&path).unwrap();
            Runtime(path)
        }

        fn ours(&self) -> PathBuf {
            self.0.join(NAME)
        }
    }

    impl Drop for Runtime {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn scripts_are_private_files_of_their_own() {
        let runtime = Runtime::new();
        let dir = ScriptDir::open(&runtime.0).unwrap();
        assert_eq!(mode(&runtime.ours()), 0o700);
        let token = Token::new(&mut fastrand::Rng::with_seed(3));
        let name = Script::Layout.file_name(&token);
        let path = dir.write(&name, "workspace;").unwrap();
        assert_eq!(path, runtime.ours().join(&name));
        assert_eq!(fs::read_to_string(&path).unwrap(), "workspace;");
        assert_eq!(mode(&path), 0o600);
        // A name is never written twice.
        assert!(dir.write(&name, "again").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "workspace;");
    }

    #[test]
    fn a_symlink_in_the_files_place_is_not_followed() {
        let runtime = Runtime::new();
        let dir = ScriptDir::open(&runtime.0).unwrap();
        let target = runtime.0.join("elsewhere");
        fs::write(&target, "kept").unwrap();
        symlink(&target, runtime.ours().join("planted.js")).unwrap();
        assert!(dir.write("planted.js", "script").is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "kept");
    }

    #[test]
    fn a_directory_open_to_others_or_a_symlink_is_not_used() {
        let runtime = Runtime::new();
        fs::create_dir(runtime.ours()).unwrap();
        fs::set_permissions(runtime.ours(), fs::Permissions::from_mode(0o750)).unwrap();
        let err = ScriptDir::open(&runtime.0).unwrap_err();
        assert!(err.contains("open to other users"), "{err}");

        // A private file of this user's in the directory's place.
        fs::remove_dir(runtime.ours()).unwrap();
        fs::write(runtime.ours(), "").unwrap();
        fs::set_permissions(runtime.ours(), fs::Permissions::from_mode(0o600)).unwrap();
        assert!(ScriptDir::open(&runtime.0).is_err());
        fs::remove_file(runtime.ours()).unwrap();
        fs::create_dir(runtime.ours()).unwrap();

        // A private directory of this user's, reached through a symlink:
        // only the symlink keeps it out.
        let elsewhere = Runtime::new();
        fs::set_permissions(&elsewhere.0, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir(runtime.ours()).unwrap();
        symlink(&elsewhere.0, runtime.ours()).unwrap();
        assert!(ScriptDir::open(&runtime.0).is_err());
    }

    #[test]
    fn a_sweep_removes_the_scripts_files_and_nothing_else() {
        let runtime = Runtime::new();
        let dir = ScriptDir::open(&runtime.0).unwrap();
        let mut rng = fastrand::Rng::with_seed(5);
        let layout = Script::Layout.file_name(&Token::new(&mut rng));
        let position = Script::Position.file_name(&Token::new(&mut rng));
        for name in [&layout, &position, "notes.txt", "auto-clicker-layoutx.js"] {
            fs::write(runtime.ours().join(name), "").unwrap();
        }
        let left = || {
            let mut names: Vec<String> = fs::read_dir(runtime.ours())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        };
        dir.sweep(Some(Script::Position));
        assert_eq!(
            left(),
            [layout.as_str(), "auto-clicker-layoutx.js", "notes.txt"]
        );
        dir.sweep(None);
        assert_eq!(left(), ["auto-clicker-layoutx.js", "notes.txt"]);
    }
}
