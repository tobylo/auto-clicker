//! Whether the desktop offers its shortcuts to the program: a check at
//! start that opens no session and shows no dialog.

use ashpd::desktop::global_shortcuts::GlobalShortcuts;
use auto_clicker_core::input::InputError;
use tokio::time::timeout;

use crate::session::{CALL_TIMEOUT, Portal};

/// The first GlobalShortcuts version with `ConfigureShortcuts`, which the
/// program's hotkeys are set with on this route.
const MIN_VERSION: u32 = 2;

/// Checks that the desktop's GlobalShortcuts portal can serve the program,
/// on a connection that registered the app id first, as every portal call
/// must here. The connection closes when this returns.
///
/// # Errors
/// [`InputError::PortalUnavailable`] when the desktop has no such portal,
/// only an older one, or the app id cannot be registered: nothing changes
/// that before the next login. [`InputError::Portal`] when the bus or the
/// portal did not answer in time or a call failed: it may answer later.
pub async fn shortcuts_offered() -> Result<(), InputError> {
    let portal = Portal::connect().await?;
    let proxy = timeout(
        CALL_TIMEOUT,
        GlobalShortcuts::with_connection(portal.connection().clone()),
    )
    .await
    .map_err(|_| {
        InputError::Portal(format!(
            "GlobalShortcuts: no answer within {} s",
            CALL_TIMEOUT.as_secs()
        ))
    })?
    .map_err(not_offered)?;
    version_offered(proxy.version())
}

fn version_offered(version: u32) -> Result<(), InputError> {
    if version >= MIN_VERSION {
        Ok(())
    } else {
        Err(too_old(version))
    }
}

fn too_old(version: u32) -> InputError {
    InputError::PortalUnavailable(format!(
        "GlobalShortcuts version {version} is too old; version {MIN_VERSION} is needed"
    ))
}

/// What a failure to reach the portal means: absent or too old, or a call
/// that failed.
fn not_offered(err: ashpd::Error) -> InputError {
    match err {
        ashpd::Error::PortalNotFound(_) => {
            InputError::PortalUnavailable("the desktop has no GlobalShortcuts portal".to_owned())
        }
        ashpd::Error::RequiresVersion(_, found) => too_old(found),
        other => InputError::Portal(format!("GlobalShortcuts: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_two_is_needed() {
        assert!(matches!(
            version_offered(1),
            Err(InputError::PortalUnavailable(_))
        ));
        assert!(version_offered(2).is_ok());
        assert!(version_offered(3).is_ok());
    }

    #[test]
    fn an_absent_or_old_portal_is_unavailable_and_a_failed_call_is_not() {
        let name = zbus::names::OwnedInterfaceName::try_from("org.freedesktop.portal.X").unwrap();
        assert!(matches!(
            not_offered(ashpd::Error::PortalNotFound(name)),
            InputError::PortalUnavailable(_)
        ));
        assert!(matches!(
            not_offered(ashpd::Error::RequiresVersion(2, 1)),
            InputError::PortalUnavailable(why) if why.contains("version 1")
        ));
        assert!(matches!(
            not_offered(ashpd::Error::NoResponse),
            InputError::Portal(_)
        ));
    }
}
