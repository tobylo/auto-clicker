//! Why synthetic input failed, as the tray and the window show it.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::Quoted;
use crate::input::InputError;

/// An [`InputError`] as a message: what went wrong, with the operating
/// system's or the portal's own words quoted.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum InputFailure {
    NoPermission(Quoted),
    UinputMissing(Quoted),
    CreateDevice(Quoted),
    Write(Quoted),
    PortalUnavailable(Quoted),
    NotAllowed,
    AllowedNow,
    Portal(Quoted),
    SessionLost(Quoted),
}

impl From<&InputError> for InputFailure {
    fn from(err: &InputError) -> InputFailure {
        let quoted = |text: &dyn fmt::Display| Quoted::new(text.to_string());
        match err {
            InputError::NoPermission(e) => InputFailure::NoPermission(quoted(e)),
            InputError::UinputMissing(e) => InputFailure::UinputMissing(quoted(e)),
            InputError::CreateDevice(e) => InputFailure::CreateDevice(quoted(e)),
            InputError::Write(e) => InputFailure::Write(quoted(e)),
            InputError::PortalUnavailable(why) => InputFailure::PortalUnavailable(quoted(why)),
            InputError::NotAllowed => InputFailure::NotAllowed,
            InputError::AllowedNow => InputFailure::AllowedNow,
            InputError::Portal(why) => InputFailure::Portal(quoted(why)),
            InputError::SessionLost(why) => InputFailure::SessionLost(quoted(why)),
        }
    }
}

impl fmt::Display for InputFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputFailure::NoPermission(e) => write!(
                f,
                "no permission to open /dev/uinput ({e}); install the package's udev rule \
                 and log in again, or add your user to the `input` group"
            ),
            InputFailure::UinputMissing(e) => write!(
                f,
                "/dev/uinput is not available ({e}); load the kernel module with \
                 `sudo modprobe uinput`"
            ),
            InputFailure::CreateDevice(e) => {
                write!(f, "could not create a virtual input device: {e}")
            }
            InputFailure::Write(e) => write!(f, "could not write to a virtual input device: {e}"),
            InputFailure::PortalUnavailable(why) => {
                write!(f, "the desktop's remote input portal cannot be used: {why}")
            }
            InputFailure::NotAllowed => {
                f.write_str("not allowed in the desktop's dialog; Start asks again")
            }
            InputFailure::AllowedNow => f.write_str("allowed; press Start again"),
            InputFailure::Portal(why) => write!(f, "the remote input portal failed: {why}"),
            InputFailure::SessionLost(why) => write!(f, "the desktop ended remote input: {why}"),
        }
    }
}
