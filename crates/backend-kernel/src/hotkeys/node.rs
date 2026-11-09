//! Opening one `/dev/input` node for hotkeys.
//!
//! The node is opened read-only, and every check runs on that open file: its
//! device number gives the sysfs device that tells this program's own
//! devices apart, and the same descriptor becomes the event stream. Nothing
//! is looked up again by path after the open.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use auto_clicker_core::hotkeys::Bindings;
use evdev::KeyCode;
use evdev::raw_stream::{EventStream, RawDevice};

/// Where the reader looks; tests point these at a temporary directory.
#[derive(Clone, Debug)]
pub(crate) struct Paths {
    /// The evdev nodes, `/dev/input`.
    pub(crate) dev: PathBuf,
    /// udev's database, `/run/udev/data`: `c<major>:<minor>` exists once udev
    /// has set the node up.
    pub(crate) udev_data: PathBuf,
    /// Character devices by number, `/sys/dev/char`.
    pub(crate) sys_char: PathBuf,
}

impl Paths {
    pub(crate) fn system() -> Paths {
        Paths {
            dev: PathBuf::from("/dev/input"),
            udev_data: PathBuf::from("/run/udev/data"),
            sys_char: PathBuf::from("/sys/dev/char"),
        }
    }
}

/// What opening a node gave. `Keyboard` holds the stream to read.
pub(crate) enum Node {
    Keyboard(Box<EventStream>),
    Ours,
    NotKeyboard,
    Gone,
    Denied { udev_done: bool },
    Failed(String),
}

/// Whether an error means the device went away (no node, or `ENODEV` and
/// `ENXIO`: a node without its device), rather than that it cannot be used.
pub(crate) fn is_gone(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::NotFound
        || matches!(err.raw_os_error(), Some(libc::ENODEV | libc::ENXIO))
}

/// Opens the node at `path` and decides what it is. `own` holds the sysfs
/// paths of this program's virtual devices.
pub(crate) fn open(path: &Path, paths: &Paths, own: &[PathBuf], bindings: &Bindings) -> Node {
    match open_file(path) {
        Ok(file) => classify(file, paths, own, bindings).unwrap_or_else(|err| failed(&err)),
        Err(err) => open_failed(&err, path, paths),
    }
}

/// Opens read-only, never blocking (a FIFO in its place would block an
/// ordinary open) and never following a symlink to some other device.
fn open_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
}

fn classify(file: File, paths: &Paths, own: &[PathBuf], bindings: &Bindings) -> io::Result<Node> {
    let meta = file.metadata()?;
    if !meta.file_type().is_char_device() {
        return Ok(Node::NotKeyboard);
    }
    // A failed lookup skips the device: it never counts as "not ours".
    if own.contains(&input_device(paths, meta.rdev())?) {
        return Ok(Node::Ours);
    }
    let device = RawDevice::from_fd(file.into())?;
    let wanted = device
        .supported_keys()
        .is_some_and(|keys| bindings.wants_device(|code| keys.contains(KeyCode(code))));
    if !wanted {
        return Ok(Node::NotKeyboard);
    }
    use_monotonic_clock(device.as_fd())?;
    Ok(Node::Keyboard(Box::new(device.into_event_stream()?)))
}

/// `EVIOCSCLOCKID`: `_IOW('E', 0xa0, int)`.
const EVIOCSCLOCKID: u64 = (1 << 30) | (4 << 16) | (0x45 << 8) | 0xa0;

/// Makes the device stamp its events with the monotonic clock, so events
/// from several devices keep their order when the wall clock is set.
#[allow(unsafe_code)]
fn use_monotonic_clock(fd: BorrowedFd<'_>) -> io::Result<()> {
    let clock: libc::c_int = libc::CLOCK_MONOTONIC;
    // SAFETY: `fd` is borrowed, so it is open for this call. The request
    // reads one `int` through the pointer, which points at `clock`, alive
    // until the call returns; nothing is written back.
    let result = unsafe { libc::ioctl(fd.as_raw_fd(), EVIOCSCLOCKID as _, &clock) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn failed(err: &io::Error) -> Node {
    if is_gone(err) {
        Node::Gone
    } else {
        Node::Failed(err.to_string())
    }
}

fn open_failed(err: &io::Error, path: &Path, paths: &Paths) -> Node {
    if err.kind() == io::ErrorKind::PermissionDenied {
        return Node::Denied {
            udev_done: udev_done(path, paths),
        };
    }
    failed(err)
}

/// Whether udev has finished with the node. The node's number comes from
/// the path, which is all there is without an open descriptor; a node
/// replaced meanwhile only makes the answer about the new one.
fn udev_done(path: &Path, paths: &Paths) -> bool {
    fs::metadata(path)
        .map(|meta| {
            let (major, minor) = (libc::major(meta.rdev()), libc::minor(meta.rdev()));
            paths.udev_data.join(format!("c{major}:{minor}")).exists()
        })
        .unwrap_or(false)
}

/// The sysfs input device (`/sys/devices/.../input/inputN`) behind the
/// event node with this device number: `/sys/dev/char/M:m` links to its
/// `eventN` directory, whose parent is the input device.
fn input_device(paths: &Paths, device_number: u64) -> io::Result<PathBuf> {
    let (major, minor) = (libc::major(device_number), libc::minor(device_number));
    let event_dir = fs::canonicalize(paths.sys_char.join(format!("{major}:{minor}")))?;
    event_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unplugged_devices_are_gone_and_other_errors_are_not() {
        let errno = io::Error::from_raw_os_error;
        assert!(is_gone(&errno(libc::ENODEV)));
        assert!(is_gone(&errno(libc::ENXIO)));
        assert!(is_gone(&errno(libc::ENOENT)));
        assert!(!is_gone(&errno(libc::EACCES)));
        assert!(!is_gone(&errno(libc::EIO)));
        assert!(!is_gone(&errno(libc::ENOTTY)));
    }

    #[test]
    fn the_input_device_is_the_parent_of_the_event_directory() {
        let root = std::env::temp_dir().join(format!(
            "auto-clicker-node-{}-{:016x}",
            std::process::id(),
            fastrand::u64(..)
        ));
        let event_dir = root.join("devices/input7/event7");
        fs::create_dir_all(&event_dir).unwrap();
        fs::create_dir(root.join("char")).unwrap();
        std::os::unix::fs::symlink(&event_dir, root.join("char/13:71")).unwrap();
        let paths = Paths {
            dev: root.join("input"),
            udev_data: root.join("udev"),
            sys_char: root.join("char"),
        };
        let found = input_device(&paths, libc::makedev(13, 71));
        let missing = input_device(&paths, libc::makedev(13, 72));
        let expected = fs::canonicalize(&root).unwrap().join("devices/input7");
        let _ = fs::remove_dir_all(&root);
        let results = (
            found.map_err(|err| err.kind()),
            missing.map_err(|err| err.kind()),
        );
        assert_eq!(results, (Ok(expected), Err(io::ErrorKind::NotFound)));
    }

    #[test]
    fn the_clock_request_is_the_kernels() {
        // From the kernel's uapi headers, include/uapi/linux/input.h.
        assert_eq!(EVIOCSCLOCKID, 0x4004_45a0);
    }
}
