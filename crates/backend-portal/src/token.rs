//! The RemoteDesktop restore token: what lets a start skip the permission
//! dialog. It lives in `$XDG_STATE_HOME/auto-clicker/portal-token`.
//!
//! The file is parsed once into a [`RestoreToken`]. Anything else at that
//! path (a directory, a FIFO, a symlink, a file too large or holding
//! something that is not a token) is reported and ignored, so the dialog
//! shows again; it is never half-read. The token is written atomically.

use std::ffi::OsString;
use std::fmt;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use auto_clicker_core::fs::atomic_write;

/// The longest token accepted. Portals hand out UUIDs (36 bytes); this
/// leaves room for any other form without reading an unbounded file.
const MAX_LEN: usize = 512;

/// A restore token as the portal handed it out: printable ASCII, no spaces.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct RestoreToken(String);

impl RestoreToken {
    /// The token in `text`, which may end with one newline.
    pub(crate) fn parse(text: &str) -> Option<RestoreToken> {
        let token = text.strip_suffix('\n').unwrap_or(text);
        let valid = !token.is_empty()
            && token.len() <= MAX_LEN
            && token.bytes().all(|byte| byte.is_ascii_graphic());
        valid.then(|| RestoreToken(token.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The token is a credential of sorts: it never appears in a log.
impl fmt::Debug for RestoreToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RestoreToken(..)")
    }
}

/// Where the token lives: `$XDG_STATE_HOME/auto-clicker/portal-token`, or
/// `~/.local/state/auto-clicker/portal-token` when `XDG_STATE_HOME` is unset
/// or not absolute. `None` when neither gives an absolute directory.
pub fn token_path() -> Option<PathBuf> {
    token_path_from(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

fn token_path_from(xdg_state_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let absolute = |dir: OsString| Some(PathBuf::from(dir)).filter(|dir| dir.is_absolute());
    xdg_state_home
        .and_then(absolute)
        .or_else(|| {
            home.and_then(absolute)
                .map(|home| home.join(".local").join("state"))
        })
        .map(|dir| dir.join("auto-clicker").join("portal-token"))
}

/// Reads the token at `path`: `Ok(None)` when there is no file.
///
/// The file is opened without following a symlink and without blocking (a
/// FIFO would otherwise hang the daemon's one thread), then checked on the
/// open handle: a regular file of at most [`MAX_LEN`] bytes, holding a
/// token.
pub(crate) fn read(path: &Path) -> io::Result<Option<RestoreToken>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    if !file.metadata()?.is_file() {
        return Err(invalid("not a regular file"));
    }
    let text = read_bounded(file)?;
    RestoreToken::parse(&text)
        .map(Some)
        .ok_or_else(|| invalid("it does not hold a restore token"))
}

/// At most `MAX_LEN` bytes and a newline; longer is an error, not a cut.
fn read_bounded(file: File) -> io::Result<String> {
    let limit = MAX_LEN.saturating_add(2);
    let mut bytes = Vec::new();
    file.take(u64::try_from(limit).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)?;
    if bytes.len() >= limit {
        return Err(invalid("too large"));
    }
    String::from_utf8(bytes).map_err(|_| invalid("not text"))
}

fn invalid(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

/// Saves `token` at `path` atomically, creating its directory private to
/// the user (0700) when it is missing.
pub(crate) fn save(path: &Path, token: &RestoreToken) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let mut text = token.as_str().to_owned();
    text.push('\n');
    atomic_write(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    /// A fresh directory under the system's temp directory, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> TempDir {
            let dir = std::env::temp_dir().join(format!(
                "auto-clicker-token-{}-{:016x}",
                std::process::id(),
                fastrand::u64(..)
            ));
            fs::create_dir(&dir).unwrap();
            TempDir(dir)
        }

        fn token(&self) -> PathBuf {
            self.0.join("auto-clicker").join("portal-token")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn token(text: &str) -> RestoreToken {
        RestoreToken::parse(text).unwrap()
    }

    fn write_raw(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_saved_token_reads_back() {
        let dir = TempDir::new();
        let path = dir.token();
        let saved = token("2f1c4f5e-6a8b-4b5e-9a57-1d2c3b4a5f60");

        save(&path, &saved).unwrap();

        assert_eq!(read(&path).unwrap(), Some(saved));
        let dir_mode = fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        let file_mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!((dir_mode & 0o777, file_mode & 0o777), (0o700, 0o600));
    }

    #[test]
    fn a_new_token_replaces_the_old_one_whole() {
        let dir = TempDir::new();
        let path = dir.token();
        save(&path, &token("a-much-longer-old-token-than-the-new-one")).unwrap();

        save(&path, &token("new")).unwrap();

        assert_eq!(read(&path).unwrap(), Some(token("new")));
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn no_file_is_no_token() {
        let dir = TempDir::new();
        assert_eq!(read(&dir.token()).unwrap(), None);
    }

    #[test]
    fn a_crash_mid_write_leaves_the_old_token_readable() {
        // What a crash leaves: the old file, and a temp file beside it.
        let dir = TempDir::new();
        let path = dir.token();
        save(&path, &token("old-token")).unwrap();
        fs::write(path.with_file_name(".portal-token.1.0.tmp"), b"half").unwrap();

        assert_eq!(read(&path).unwrap(), Some(token("old-token")));
    }

    #[test]
    fn a_bad_token_file_is_rejected_not_half_read() {
        let dir = TempDir::new();
        let path = dir.token();
        let too_long = "x".repeat(MAX_LEN + 1);
        for (case, bytes) in [
            ("empty", &b""[..]),
            ("a space", b"two words"),
            ("two lines", b"one\ntwo\n"),
            ("not text", b"\xff\xfe"),
            ("too long", too_long.as_bytes()),
        ] {
            write_raw(&path, bytes);
            let err = read(&path).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{case}: {err}");
        }
    }

    #[test]
    fn the_longest_token_is_accepted() {
        let dir = TempDir::new();
        let path = dir.token();
        let longest = "x".repeat(MAX_LEN);
        write_raw(&path, format!("{longest}\n").as_bytes());
        assert_eq!(read(&path).unwrap(), Some(token(&longest)));
    }

    #[test]
    fn only_a_regular_file_is_read() {
        let dir = TempDir::new();
        let path = dir.token();
        fs::create_dir_all(&path).unwrap();
        assert!(read(&path).is_err(), "a directory");
        fs::remove_dir(&path).unwrap();

        let target = dir.0.join("elsewhere");
        fs::write(&target, b"token\n").unwrap();
        symlink(&target, &path).unwrap();
        assert!(read(&path).is_err(), "a symlink");
    }

    #[test]
    fn a_fifo_is_rejected_without_blocking() {
        let dir = TempDir::new();
        let path = dir.token();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let status = std::process::Command::new("mkfifo").arg(&path).status();
        if !status.is_ok_and(|status| status.success()) {
            return;
        }
        // A blocking open would hang here, with no writer on the FIFO.
        let err = read(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{err}");
    }

    #[test]
    fn the_token_is_never_shown_in_debug_output() {
        assert_eq!(format!("{:?}", token("secret")), "RestoreToken(..)");
    }

    #[test]
    fn the_path_follows_xdg_state_home_then_home() {
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(
            token_path_from(os("/state"), os("/home/me")),
            Some(PathBuf::from("/state/auto-clicker/portal-token"))
        );
        assert_eq!(
            token_path_from(os("relative"), os("/home/me")),
            Some(PathBuf::from(
                "/home/me/.local/state/auto-clicker/portal-token"
            ))
        );
        assert_eq!(
            token_path_from(None, os("/home/me")),
            Some(PathBuf::from(
                "/home/me/.local/state/auto-clicker/portal-token"
            ))
        );
        assert_eq!(token_path_from(None, os("relative")), None);
        assert_eq!(token_path_from(None, None), None);
    }
}
