//! Telling the desktop's own "no" from a call that got no answer.
//!
//! The routes are picked once at start, so the difference decides the
//! daemon's whole session: a portal that answered that it cannot serve the
//! program (no such interface, too old, no desktop file for the app id)
//! stays so until the next login, while one that did not answer in time,
//! or whose bus connection failed, may answer the next call.

use auto_clicker_core::input::InputError;
use zbus::fdo::Error as Fdo;
use zbus::names::InterfaceName;

use crate::session::call;

/// The portal's bus name and object.
const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";

/// Whether `err` is an answer that the call cannot succeed, rather than no
/// answer: the bus or the service replied with an error of its own.
pub(crate) fn refused(err: &Fdo) -> bool {
    !matches!(
        err,
        Fdo::NoReply(_)
            | Fdo::Timeout(_)
            | Fdo::TimedOut(_)
            | Fdo::Disconnected(_)
            | Fdo::NoServer(_)
            | Fdo::IOError(_)
            | Fdo::NoMemory(_)
            | Fdo::LimitsExceeded(_)
            | Fdo::ZBus(_)
    )
}

/// `err` from a call named `what`, as the user sees it: unavailable when
/// the desktop said no, a failed call otherwise.
pub(crate) fn classify(what: &str, err: Fdo) -> InputError {
    if refused(&err) {
        InputError::PortalUnavailable(format!("{what}: {err}"))
    } else {
        InputError::Portal(format!("{what}: {err}"))
    }
}

/// The version of the portal's `interface`, read on `connection`, which
/// must have registered the app id. ashpd reads it too, but takes any
/// failure for version 1, which would read as "too old" for good.
pub(crate) async fn version(
    connection: &zbus::Connection,
    interface: &'static str,
) -> Result<u32, InputError> {
    let what = format!("{interface} version");
    let name = InterfaceName::from_static_str(interface)
        .map_err(|err| InputError::PortalUnavailable(format!("{what}: {err}")))?;
    let read = async {
        let properties = zbus::fdo::PropertiesProxy::builder(connection)
            .destination(DESKTOP)?
            .path(DESKTOP_PATH)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        properties.get(name, "version").await
    };
    let value = call(&what, read)
        .await?
        .map_err(|err| classify(&what, err))?;
    u32::try_from(value)
        .map_err(|err| InputError::PortalUnavailable(format!("{what} is not a number: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_with_an_error_is_a_refusal_and_no_reply_is_not() {
        let said_no = [
            Fdo::InvalidArgs("No such interface".to_owned()),
            Fdo::ServiceUnknown("no portal".to_owned()),
            Fdo::UnknownMethod("Register".to_owned()),
            Fdo::Failed("App info not found".to_owned()),
            Fdo::AccessDenied("no".to_owned()),
        ];
        for err in said_no {
            assert!(refused(&err), "{err}");
            assert!(matches!(
                classify("x", err),
                InputError::PortalUnavailable(_)
            ));
        }
        let no_answer = [
            Fdo::NoReply("gone".to_owned()),
            Fdo::Timeout("slow".to_owned()),
            Fdo::TimedOut("slow".to_owned()),
            Fdo::Disconnected("bus".to_owned()),
            Fdo::NoServer("bus".to_owned()),
            Fdo::IOError("pipe".to_owned()),
            Fdo::NoMemory("bus".to_owned()),
            Fdo::LimitsExceeded("bus".to_owned()),
            Fdo::ZBus(zbus::Error::InterfaceNotFound),
        ];
        for err in no_answer {
            assert!(!refused(&err), "{err}");
            assert!(matches!(classify("x", err), InputError::Portal(_)));
        }
    }
}
