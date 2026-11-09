//! The one error type of the input layer.

use std::{fmt, io};

/// Why synthetic input could not be set up or written.
#[derive(Debug)]
pub enum InputError {
    /// The program may not open `/dev/uinput`.
    NoPermission(io::Error),
    /// `/dev/uinput` does not exist, or the `uinput` module is not loaded.
    UinputMissing(io::Error),
    /// A virtual device could not be configured or created.
    CreateDevice(io::Error),
    /// Writing an event to a device failed.
    Write(io::Error),
}

impl InputError {
    /// The underlying operating-system error.
    pub fn io_error(&self) -> &io::Error {
        match self {
            InputError::NoPermission(e)
            | InputError::UinputMissing(e)
            | InputError::CreateDevice(e)
            | InputError::Write(e) => e,
        }
    }
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputError::NoPermission(e) => write!(
                f,
                "no permission to open /dev/uinput ({e}); install the package's udev rule \
                 and log in again, or add your user to the `input` group"
            ),
            InputError::UinputMissing(e) => write!(
                f,
                "/dev/uinput is not available ({e}); load the kernel module with \
                 `sudo modprobe uinput`"
            ),
            InputError::CreateDevice(e) => {
                write!(f, "could not create a virtual input device: {e}")
            }
            InputError::Write(e) => write!(f, "could not write to a virtual input device: {e}"),
        }
    }
}

impl std::error::Error for InputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.io_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_fix() {
        let denied = InputError::NoPermission(io::ErrorKind::PermissionDenied.into());
        let missing = InputError::UinputMissing(io::ErrorKind::NotFound.into());
        assert!(denied.to_string().contains("input` group"));
        assert!(missing.to_string().contains("modprobe uinput"));
    }

    #[test]
    fn source_is_the_io_error() {
        use std::error::Error;
        let err = InputError::Write(io::Error::other("boom"));
        assert_eq!(
            err.source().map(ToString::to_string),
            Some("boom".to_owned())
        );
    }
}
