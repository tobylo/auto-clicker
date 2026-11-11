//! Whether the desktop offers its shortcuts to the program: a check at
//! start that opens no session and shows no dialog.

use auto_clicker_core::input::InputError;

use crate::answer::version;
use crate::session::Portal;

/// The interface hotkeys come through.
const GLOBAL_SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";
/// The first GlobalShortcuts version with `ConfigureShortcuts`, which the
/// program's hotkeys are set with on this route.
const MIN_VERSION: u32 = 2;

/// Checks that the desktop's GlobalShortcuts portal can serve the program,
/// on a connection that registered the app id first, as every portal call
/// must here. The connection closes when this returns.
///
/// # Errors
/// [`InputError::PortalUnavailable`] when the desktop answered that it
/// cannot: no such portal, only an older one, or no app id for the
/// program. Nothing changes that before the next login.
/// [`InputError::Portal`] when the bus or the portal did not answer in time
/// or the connection failed: it may answer later.
pub async fn shortcuts_offered() -> Result<(), InputError> {
    let portal = Portal::connect().await?;
    version_offered(version(portal.connection(), GLOBAL_SHORTCUTS).await?)
}

fn version_offered(version: u32) -> Result<(), InputError> {
    if version >= MIN_VERSION {
        Ok(())
    } else {
        Err(InputError::PortalUnavailable(format!(
            "GlobalShortcuts version {version} is too old; version {MIN_VERSION} is needed"
        )))
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
}
