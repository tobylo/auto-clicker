//! The one error type of the input layer.

use std::{fmt, io};

use crate::message::InputFailure;

/// Why synthetic input could not be set up or written.
///
/// The first four come from the kernel backend (`/dev/uinput`), the rest
/// from the portal backend (the RemoteDesktop portal and libei).
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
    /// The desktop has no usable RemoteDesktop portal, or would not take the
    /// program's app id. Says what is missing.
    PortalUnavailable(String),
    /// The user did not allow input in the portal's dialog: Deny, Escape or
    /// closing it. A later start asks again.
    NotAllowed,
    /// The user allowed input in the portal's dialog just now. The run does
    /// not begin, since the dialog took the focus from the game; the next
    /// start goes ahead without a dialog.
    AllowedNow,
    /// A portal call failed or did not answer in time. Says which, and why.
    Portal(String),
    /// The compositor ended the input session, or paused its devices. Says
    /// why. Whatever was held is released by the compositor.
    SessionLost(String),
}

impl InputError {
    /// The underlying operating-system error, for the kernel backend's
    /// errors; the portal backend's carry none.
    pub fn io_error(&self) -> Option<&io::Error> {
        match self {
            InputError::NoPermission(e)
            | InputError::UinputMissing(e)
            | InputError::CreateDevice(e)
            | InputError::Write(e) => Some(e),
            InputError::PortalUnavailable(_)
            | InputError::NotAllowed
            | InputError::AllowedNow
            | InputError::Portal(_)
            | InputError::SessionLost(_) => None,
        }
    }
}

/// The same words the tray shows, in English: see [`InputFailure`].
impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        InputFailure::from(self).fmt(f)
    }
}

impl std::error::Error for InputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.io_error()
            .map(|err| err as &(dyn std::error::Error + 'static))
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
        assert!(
            InputError::NotAllowed
                .to_string()
                .contains("Start asks again")
        );
        assert!(
            InputError::AllowedNow
                .to_string()
                .contains("press Start again")
        );
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

    #[test]
    fn portal_errors_have_no_source() {
        use std::error::Error;
        let errors = [
            InputError::PortalUnavailable("no portal".to_owned()),
            InputError::NotAllowed,
            InputError::AllowedNow,
            InputError::Portal("Start: timed out".to_owned()),
            InputError::SessionLost("disconnected".to_owned()),
        ];
        assert!(errors.iter().all(|err| err.source().is_none()));
    }
}
