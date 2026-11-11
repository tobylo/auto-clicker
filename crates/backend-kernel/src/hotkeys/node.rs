//! Opening one `/dev/input` node for hotkeys or a recording.
//!
//! The node is opened read-only, and every check runs on that open file: its
//! device number gives the sysfs device that tells this program's own
//! devices apart, its capabilities say what it is, and the same descriptor
//! becomes the event stream. Nothing is looked up again by path after the
//! open.
//!
//! Keyboards with a bound key or a modifier are always read, for hotkeys.
//! While recording, mice and every other keyboard are read too. Touchpads
//! never are: their taps and clicks are not a mouse's, and cannot be played
//! back as one.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use auto_clicker_core::hotkeys::Bindings;
use auto_clicker_core::input::Key;
use evdev::raw_stream::{EventStream, RawDevice};
use evdev::{KeyCode, RelativeAxisCode};

/// Where the reader looks; tests point these at a temporary directory.
#[derive(Clone, Debug)]
pub(crate) struct Paths {
    /// The evdev nodes, `/dev/input`.
    pub(crate) dev: PathBuf,
    /// Character devices by number, `/sys/dev/char`.
    pub(crate) sys_char: PathBuf,
}

impl Paths {
    pub(crate) fn system() -> Paths {
        Paths {
            dev: PathBuf::from("/dev/input"),
            sys_char: PathBuf::from("/sys/dev/char"),
        }
    }
}

/// What opening a node gave. `Device` holds the stream to read.
pub(crate) enum Node {
    Device(Box<EventStream>, Role),
    Ours,
    NotWanted,
    Gone,
    Denied,
    Failed(String),
}

/// Why a device is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Role {
    /// It has a bound key or a modifier: read for hotkeys, always.
    pub(crate) hotkeys: bool,
    /// It is a mouse or a keyboard and no touchpad: what it does is
    /// recorded.
    pub(crate) record: bool,
}

/// What the reader wants opened now.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Wanted<'a> {
    pub(crate) bindings: &'a Bindings,
    /// A capture runs: every keyboard counts as one with a hotkey key.
    pub(crate) capture: bool,
    /// A recording runs: recordable devices are wanted too.
    pub(crate) recording: bool,
}

/// What a device can do, as far as reading it goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Capabilities {
    /// A bound key or a modifier key.
    pub(crate) hotkey_keys: bool,
    /// Any key a recording can hold.
    pub(crate) keys: bool,
    /// Relative X and Y and a left button.
    pub(crate) mouse: bool,
    /// Reports fingers (`BTN_TOOL_FINGER`): a touchpad.
    pub(crate) touchpad: bool,
}

/// `BTN_TOOL_FINGER`, which touchpads have and mice do not.
const BTN_TOOL_FINGER: u16 = 0x145;
/// `BTN_LEFT`.
const BTN_LEFT: u16 = 0x110;

/// Whether to read a device that can do `caps`, and why; `None` to leave it
/// alone.
pub(crate) fn decide(caps: Capabilities, recording: bool) -> Option<Role> {
    let role = Role {
        hotkeys: caps.hotkey_keys,
        record: !caps.touchpad && (caps.mouse || caps.keys),
    };
    (role.hotkeys || (recording && role.record)).then_some(role)
}

/// Whether an error means the device went away (no node, or `ENODEV` and
/// `ENXIO`: a node without its device), rather than that it cannot be used.
pub(crate) fn is_gone(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::NotFound
        || matches!(err.raw_os_error(), Some(libc::ENODEV | libc::ENXIO))
}

/// Opens the node at `path` and decides what it is. `own` holds the sysfs
/// paths of this program's virtual devices.
pub(crate) fn open(path: &Path, paths: &Paths, own: &[PathBuf], wanted: Wanted<'_>) -> Node {
    match open_file(path) {
        Ok(file) => classify(file, paths, own, wanted).unwrap_or_else(|err| failed(&err)),
        Err(err) => open_failed(&err),
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

fn classify(file: File, paths: &Paths, own: &[PathBuf], wanted: Wanted<'_>) -> io::Result<Node> {
    let meta = file.metadata()?;
    if !meta.file_type().is_char_device() {
        return Ok(Node::NotWanted);
    }
    // A failed lookup skips the device: it never counts as "not ours".
    if own.contains(&input_device(paths, meta.rdev())?) {
        return Ok(Node::Ours);
    }
    let device = RawDevice::from_fd(file.into())?;
    let Some(role) = decide(capabilities(&device, wanted), wanted.recording) else {
        return Ok(Node::NotWanted);
    };
    use_monotonic_clock(device.as_fd())?;
    Ok(Node::Device(Box::new(device.into_event_stream()?), role))
}

/// Whether a device is read for hotkeys: it has a bound key or a modifier
/// (`bound`), or a capture runs and it has any key a hotkey can use.
fn hotkey_device(bound: bool, capture: bool, any_key: bool) -> bool {
    bound || (capture && any_key)
}

fn capabilities(device: &RawDevice, wanted: Wanted<'_>) -> Capabilities {
    let Some(keys) = device.supported_keys() else {
        return Capabilities::default();
    };
    let has = |code| keys.contains(KeyCode(code));
    let relative = device.supported_relative_axes();
    let has_axis = |axis| relative.is_some_and(|axes| axes.contains(axis));
    let any_key = Key::CODES.into_iter().any(has);
    Capabilities {
        hotkey_keys: hotkey_device(wanted.bindings.wants_device(has), wanted.capture, any_key),
        keys: any_key,
        mouse: has(BTN_LEFT)
            && has_axis(RelativeAxisCode::REL_X)
            && has_axis(RelativeAxisCode::REL_Y),
        touchpad: has(BTN_TOOL_FINGER),
    }
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

fn open_failed(err: &io::Error) -> Node {
    if err.kind() == io::ErrorKind::PermissionDenied {
        Node::Denied
    } else {
        failed(err)
    }
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
    fn a_capture_reads_every_keyboard_and_otherwise_only_hotkey_ones() {
        // (bound, capture, any key) and whether it is read for hotkeys.
        let cases = [
            ((true, false, true), true),
            ((false, false, true), false),
            ((false, true, true), true),
            ((false, true, false), false),
            ((true, true, false), true),
        ];
        for ((bound, capture, any_key), read) in cases {
            assert_eq!(
                hotkey_device(bound, capture, any_key),
                read,
                "{bound} {capture} {any_key}"
            );
        }
    }

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
    fn opening_a_fifo_in_place_of_a_node_does_not_block() {
        let dir = std::env::temp_dir().join(format!(
            "auto-clicker-fifo-{}-{:016x}",
            std::process::id(),
            fastrand::u64(..)
        ));
        fs::create_dir(&dir).unwrap();
        let fifo = dir.join("event6");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        // On a thread of its own: a blocking open never returns, and the
        // test must fail at its deadline rather than hang.
        let (done, opened) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(open_file(&fifo).is_ok());
        });
        let result = opened.recv_timeout(std::time::Duration::from_secs(5));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(result, Ok(true), "the open blocked or failed");
    }

    #[test]
    fn hotkey_keyboards_are_always_read_and_the_rest_only_while_recording() {
        let caps = |hotkey_keys, keys, mouse, touchpad| Capabilities {
            hotkey_keys,
            keys,
            mouse,
            touchpad,
        };
        let role = |hotkeys, record| Some(Role { hotkeys, record });
        let cases = [
            // (capabilities, idle, recording)
            (
                caps(true, true, false, false),
                role(true, true),
                role(true, true),
            ),
            (caps(false, false, true, false), None, role(false, true)),
            (caps(false, true, false, false), None, role(false, true)),
            (caps(false, true, true, false), None, role(false, true)),
            // A touchpad: never recorded, its keys read only for hotkeys.
            (caps(false, false, false, true), None, None),
            (caps(false, true, true, true), None, None),
            (
                caps(true, true, false, true),
                role(true, false),
                role(true, false),
            ),
            // A tablet, a joystick: nothing to read.
            (caps(false, false, false, false), None, None),
        ];
        for (caps, idle, recording) in cases {
            assert_eq!(decide(caps, false), idle, "{caps:?} idle");
            assert_eq!(decide(caps, true), recording, "{caps:?} recording");
        }
    }

    #[test]
    fn the_clock_request_is_the_kernels() {
        // From the kernel's uapi headers, include/uapi/linux/input.h.
        assert_eq!(EVIOCSCLOCKID, 0x4004_45a0);
    }
}
