//! The file calls that differ on Linux.

use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use nix::fcntl::{RenameFlags, renameat2};

use super::Link;
use super::patience::{Op, Patience};

/// Linux has no sharing violations: no failure is tried again.
pub(super) fn transient(_err: &io::Error, _op: Op) -> bool {
    false
}

/// A new file at `path`, readable and writable by its owner only.
pub(super) fn create_temp(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// Renames `temp` to `path` only if nothing is there: `RENAME_NOREPLACE`, so
/// a file that appears meanwhile is never overwritten.
pub(super) fn place_new(temp: &Path, path: &Path, _patience: &Patience) -> io::Result<()> {
    renameat2(None, temp, None, path, RenameFlags::RENAME_NOREPLACE).map_err(io::Error::from)
}

/// Opens a directory to flush it; anything else in its place (a FIFO,
/// which would block an ordinary open) fails at once.
pub(super) fn sync_dir(dir: &Path) -> io::Result<()> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NONBLOCK)
        .open(dir)?
        .sync_all()
}

pub(super) fn sync_open_dir(dir: &File) -> io::Result<()> {
    dir.sync_all()
}

/// Opens `path` without blocking on a FIFO, and with `O_NOFOLLOW` where
/// `link` refuses one (`ELOOP` then).
pub(super) fn open_read(path: &Path, link: Link) -> io::Result<File> {
    let no_follow = match link {
        Link::Follow => 0,
        Link::Refuse => libc::O_NOFOLLOW,
    };
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | no_follow)
        .open(path)
}

/// Opens the directory `dir`, refusing a symlink in its place (`ENOTDIR`),
/// and the path the kernel resolves through the open directory, not
/// through its name.
pub(super) fn open_dir(dir: &Path) -> io::Result<(File, PathBuf)> {
    let handle = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)?;
    let held = PathBuf::from(format!("/proc/self/fd/{}", handle.as_raw_fd()));
    Ok((handle, held))
}

/// Creates `dir` and its missing ancestors with mode 0700.
pub(super) fn create_private_dir(dir: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

/// Closes the open directory `dir` to everyone but its owner, if others
/// may reach it; one this user does not own fails.
pub(super) fn make_private(dir: &File) -> io::Result<()> {
    if dir.metadata()?.permissions().mode() & 0o077 != 0 {
        dir.set_permissions(Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Where the directory held at `held` is now, as the kernel sees it; `dir`
/// if that cannot be read.
pub(super) fn now_at(held: &Path, dir: &Path) -> PathBuf {
    fs::read_link(held).unwrap_or_else(|_| dir.to_path_buf())
}
