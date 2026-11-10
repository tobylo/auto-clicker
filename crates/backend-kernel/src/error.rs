//! Tells the reasons `/dev/uinput` cannot be opened apart.

use std::io;

use auto_clicker_core::input::InputError;

/// `ENODEV` (asm-generic/errno-base.h): the device node exists but no driver
/// is behind it, as when `/dev/uinput` is static and the `uinput` module is
/// not loaded. std files it under no `ErrorKind`.
const ENODEV: i32 = 19;

/// Maps the error of opening `/dev/uinput` to what the user can do about it.
pub(crate) fn open_error(err: io::Error) -> InputError {
    match err.kind() {
        io::ErrorKind::PermissionDenied => InputError::NoPermission(err),
        io::ErrorKind::NotFound => InputError::UinputMissing(err),
        _ if err.raw_os_error() == Some(ENODEV) => InputError::UinputMissing(err),
        _ => InputError::CreateDevice(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EACCES: i32 = 13;
    const EPERM: i32 = 1;
    const ENOENT: i32 = 2;
    const EBUSY: i32 = 16;

    fn mapped(errno: i32) -> InputError {
        open_error(io::Error::from_raw_os_error(errno))
    }

    #[test]
    fn permission_errors_map_to_no_permission() {
        assert!(matches!(mapped(EACCES), InputError::NoPermission(_)));
        assert!(matches!(mapped(EPERM), InputError::NoPermission(_)));
    }

    #[test]
    fn a_missing_node_or_module_maps_to_uinput_missing() {
        assert!(matches!(mapped(ENOENT), InputError::UinputMissing(_)));
        assert!(matches!(mapped(ENODEV), InputError::UinputMissing(_)));
    }

    #[test]
    fn other_errors_keep_their_os_error() {
        let err = mapped(EBUSY);
        assert!(matches!(err, InputError::CreateDevice(_)));
        assert_eq!(
            err.io_error().and_then(io::Error::raw_os_error),
            Some(EBUSY)
        );
    }
}
