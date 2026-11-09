//! Opening one `/dev/input` node for hotkeys.
//!
//! The node is opened read-only, and every check runs on that open file: its
//! device number gives the sysfs device that tells this program's own
//! devices apart, and the same descriptor becomes the event stream. Nothing
//! is looked up again by path after the open.

use std::fs::{self, File};
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

use auto_clicker_core::hotkeys::Bindings;
use evdev::{Device, EventStream, KeyCode};

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
    Failed,
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
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => return open_failed(&err, path, paths),
    };
    let device_number = match file.metadata() {
        Ok(meta) if meta.file_type().is_char_device() => meta.rdev(),
        Ok(_) => return Node::NotKeyboard,
        Err(err) => return failed(&err),
    };
    // A failed lookup skips the device: it never counts as "not ours".
    let syspath = match input_device(paths, device_number) {
        Ok(syspath) => syspath,
        Err(err) => return failed(&err),
    };
    if own.contains(&syspath) {
        return Node::Ours;
    }
    let device = match Device::from_fd(file.into()) {
        Ok(device) => device,
        Err(err) => return failed(&err),
    };
    let wanted = device
        .supported_keys()
        .is_some_and(|keys| bindings.wants_device(|code| keys.contains(KeyCode(code))));
    if !wanted {
        return Node::NotKeyboard;
    }
    device.into_event_stream().map_or_else(
        |err| failed(&err),
        |stream| Node::Keyboard(Box::new(stream)),
    )
}

fn failed(err: &io::Error) -> Node {
    if is_gone(err) {
        Node::Gone
    } else {
        Node::Failed
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
}
