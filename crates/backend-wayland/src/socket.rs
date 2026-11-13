//! The daemon's Wayland display: its socket, opened here from
//! `WAYLAND_DISPLAY`, and the process that listens on it.
//!
//! The socket is opened by the daemon itself, never taken from an inherited
//! `WAYLAND_SOCKET`: the stream whose listener is read is the stream the
//! connection then uses.

#![allow(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::mem::size_of;
use std::num::NonZeroU32;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use auto_clicker_core::message::NoScreens;

/// Where the display's socket is: `display` as is when it is a path from
/// the root, else under `runtime`. `None` without a display, or with a
/// relative one and no runtime directory.
pub(crate) fn display_path(
    display: Option<OsString>,
    runtime: Option<OsString>,
) -> Option<PathBuf> {
    let display = display.filter(|name| !name.is_empty())?;
    let display = Path::new(&display);
    if display.is_absolute() {
        return Some(display.to_path_buf());
    }
    let runtime = runtime.filter(|dir| !dir.is_empty())?;
    Some(Path::new(&runtime).join(display))
}

/// The socket of the display the environment names.
pub(crate) fn environment_path() -> Option<PathBuf> {
    display_path(
        std::env::var_os("WAYLAND_DISPLAY"),
        std::env::var_os("XDG_RUNTIME_DIR"),
    )
}

/// Connects to the socket at `path`, without blocking the daemon's thread.
pub(crate) async fn open(path: &Path) -> Result<UnixStream, NoScreens> {
    let stream = tokio::net::UnixStream::connect(path).await.map_err(|err| {
        eprintln!(
            "auto-clicker: cannot connect to the Wayland display {}: {err}",
            shown(path.as_os_str())
        );
        NoScreens::CannotConnect
    })?;
    stream.into_std().map_err(|err| {
        eprintln!("auto-clicker: cannot use the Wayland display's socket: {err}");
        NoScreens::CannotConnect
    })
}

fn shown(path: &OsStr) -> String {
    path.to_string_lossy().into_owned()
}

/// The process that listens on `stream`'s other end: the compositor, or on
/// Plasma `kwin_wayland_wrapper`, KWin's parent (decision 0004). `None`
/// when it cannot be told: the call failed, or the process is outside this
/// daemon's process namespace, which the kernel reports as 0.
pub(crate) fn listener(stream: &UnixStream) -> Option<NonZeroU32> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = libc::socklen_t::try_from(size_of::<libc::ucred>()).ok()?;
    // SAFETY: the descriptor is `stream`'s, open for the call; the kernel
    // writes at most `length` bytes, the size of `credentials`, which
    // lives until the call returns, and sets `length` to what it wrote.
    let status = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &raw mut length,
        )
    };
    let whole = usize::try_from(length).ok() == Some(size_of::<libc::ucred>());
    if status != 0 || !whole {
        return None;
    }
    NonZeroU32::new(u32::try_from(credentials.pid).ok()?)
}

/// What is waiting on a socket to be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Waiting {
    Data,
    Nothing,
    /// The other end closed it, and everything sent was read.
    Closed,
}

/// What is waiting on `stream`, without reading it.
pub(crate) fn waiting(stream: &impl AsRawFd) -> Waiting {
    let mut byte = 0u8;
    // SAFETY: the descriptor is the caller's, open for the call; the kernel
    // writes at most one byte into `byte`, which lives until it returns.
    let read = unsafe {
        libc::recv(
            stream.as_raw_fd(),
            (&raw mut byte).cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    match read {
        0 => Waiting::Closed,
        // An error other than "would block" is the next read's to report.
        _ if read < 0
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock =>
        {
            Waiting::Nothing
        }
        _ => Waiting::Data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(text: &str) -> Option<OsString> {
        Some(OsString::from(text))
    }

    #[test]
    fn the_display_is_found_under_the_runtime_directory_or_as_a_path() {
        let cases = [
            (
                (os("wayland-1"), os("/run/user/1000")),
                Some("/run/user/1000/wayland-1"),
            ),
            (
                (os("/tmp/nested-0"), os("/run/user/1000")),
                Some("/tmp/nested-0"),
            ),
            ((os("/tmp/nested-0"), None), Some("/tmp/nested-0")),
            // No display, an empty one, or a relative one with nowhere to
            // look: none, and no guess at a name.
            ((None, os("/run/user/1000")), None),
            ((os(""), os("/run/user/1000")), None),
            ((os("wayland-1"), None), None),
            ((os("wayland-1"), os("")), None),
        ];
        for ((display, runtime), expected) in cases {
            let label = format!("{display:?} {runtime:?}");
            assert_eq!(
                display_path(display, runtime),
                expected.map(PathBuf::from),
                "{label}"
            );
        }
    }

    #[test]
    fn the_listener_of_a_socket_pair_is_this_process() {
        let (ours, _theirs) = UnixStream::pair().unwrap();
        assert_eq!(
            listener(&ours).map(NonZeroU32::get),
            Some(std::process::id())
        );
    }

    #[test]
    fn a_socket_is_closed_once_its_other_end_is_and_it_is_read_out() {
        use std::io::{Read, Write};

        let (mut ours, mut theirs) = UnixStream::pair().unwrap();
        assert_eq!(waiting(&ours), Waiting::Nothing);
        theirs.write_all(b"x").unwrap();
        drop(theirs);
        assert_eq!(waiting(&ours), Waiting::Data, "a byte is left to read");
        let mut left = Vec::new();
        ours.read_to_end(&mut left).unwrap();
        assert_eq!(waiting(&ours), Waiting::Closed);
    }

    #[tokio::test]
    async fn a_display_nobody_listens_on_cannot_be_connected() {
        let path =
            std::env::temp_dir().join(format!("auto-clicker-no-display-{}", std::process::id()));
        assert_eq!(open(&path).await.err(), Some(NoScreens::CannotConnect));
    }
}
