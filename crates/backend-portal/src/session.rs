//! The RemoteDesktop portal: the app id, the permission and the EIS socket.
//!
//! The portal scopes its stored grant by the caller's app id, and a host
//! program gets one only by calling `Registry.Register` before any other
//! portal call on its connection (X02). [`Portal::connect`] therefore opens
//! a connection of its own and registers on it first; nothing else uses a
//! connection that is not registered.

use std::future::Future;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::time::Duration;

use ashpd::desktop::remote_desktop::{
    ConnectToEISOptions, DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, ResponseError, Session};
use ashpd::enumflags2::BitFlags;
use ashpd::{AppID, PortalError};
use auto_clicker_core::input::InputError;
use tokio::time::timeout;

use crate::token::{self, RestoreToken};

/// The program's app id: the desktop file the package installs.
pub const APP_ID: &str = "io.github.tobylo.AutoClicker";

/// How long a portal call may take, apart from the dialog.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the permission dialog may stay open before the start gives up.
const DIALOG_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How long closing a session may take.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// The first RemoteDesktop version with `ConnectToEIS`.
const MIN_VERSION: u32 = 2;

/// What the program asks for.
fn wanted() -> BitFlags<DeviceType> {
    DeviceType::Keyboard | DeviceType::Pointer
}

/// A session-bus connection registered under [`APP_ID`].
#[derive(Clone, Debug)]
pub(crate) struct Portal {
    connection: zbus::Connection,
}

/// A started RemoteDesktop session and its EIS socket.
pub(crate) struct Started {
    pub(crate) session: SessionGuard,
    pub(crate) socket: OwnedFd,
    /// The device types the user allowed.
    pub(crate) devices: BitFlags<DeviceType>,
}

impl Portal {
    /// Connects to the session bus and registers the app id.
    pub(crate) async fn connect() -> Result<Portal, InputError> {
        let connection = call("session bus", zbus::Connection::session())
            .await?
            .map_err(|err| InputError::Portal(format!("session bus: {err}")))?;
        let app_id = AppID::try_from(APP_ID)
            .map_err(|err| InputError::PortalUnavailable(format!("app id {APP_ID}: {err}")))?;
        call(
            "Registry.Register",
            ashpd::register_host_app_with_connection(connection.clone(), app_id),
        )
        .await?
        .map_err(|err| {
            InputError::PortalUnavailable(format!(
                "could not register the app id {APP_ID} ({err}); is the package's \
                 {APP_ID}.desktop file installed?"
            ))
        })?;
        Ok(Portal { connection })
    }

    /// Checks, without a dialog, that the portal can serve the program.
    pub(crate) async fn probe(&self) -> Result<(), InputError> {
        let proxy = self.remote_desktop().await?;
        let available = call("AvailableDeviceTypes", proxy.available_device_types())
            .await?
            .map_err(|err| portal_error("AvailableDeviceTypes", err))?;
        if available.contains(wanted()) {
            Ok(())
        } else {
            Err(InputError::PortalUnavailable(format!(
                "it offers {available:?}, not a keyboard and a pointer"
            )))
        }
    }

    async fn remote_desktop(&self) -> Result<RemoteDesktop, InputError> {
        let proxy = call(
            "RemoteDesktop",
            RemoteDesktop::with_connection(self.connection.clone()),
        )
        .await?
        .map_err(|err| portal_error("RemoteDesktop", err))?;
        check_version(proxy.version())?;
        Ok(proxy)
    }

    /// Starts a session for a keyboard and a pointer, restoring the grant
    /// with the token at `token_path` and saving the token the portal hands
    /// back. Fails with [`InputError::AllowedNow`] when the dialog was
    /// shown, after saving the new token: the run waits for the next start.
    ///
    /// Dropping the future part way closes the session it created.
    pub(crate) async fn start(&self, token_path: Option<&Path>) -> Result<Started, InputError> {
        let proxy = self.remote_desktop().await?;
        let session = call(
            "CreateSession",
            proxy.create_session(CreateSessionOptions::default()),
        )
        .await?
        .map_err(|err| portal_error("CreateSession", err))?;
        let session = SessionGuard(Some(session));
        let sent = token_path.and_then(read_token);
        let options = SelectDevicesOptions::default()
            .set_devices(wanted())
            .set_persist_mode(PersistMode::ExplicitlyRevoked)
            .set_restore_token(sent.as_ref().map(RestoreToken::as_str));
        call("SelectDevices", async {
            proxy
                .select_devices(session.get()?, options)
                .await?
                .response()
        })
        .await?
        .map_err(|err| portal_error("SelectDevices", err))?;
        let started = timeout(DIALOG_TIMEOUT, async {
            proxy
                .start(session.get()?, None, StartOptions::default())
                .await?
                .response()
        })
        .await
        .map_err(|_| {
            InputError::Portal("the permission dialog was left open for 5 min".to_owned())
        })?
        .map_err(|err| portal_error("Start", err))?;
        let returned = started.restore_token().and_then(RestoreToken::parse);
        let saved = match (token_path, &returned) {
            (Some(path), Some(token)) => save_token(path, token),
            _ => false,
        };
        if wait_for_next_start(sent.as_ref(), saved) {
            return Err(InputError::AllowedNow);
        }
        let socket = call("ConnectToEIS", async {
            proxy
                .connect_to_eis(session.get()?, ConnectToEISOptions::default())
                .await
        })
        .await?
        .map_err(|err| portal_error("ConnectToEIS", err))?;
        Ok(Started {
            session,
            socket,
            devices: started.devices(),
        })
    }
}

/// Whether the run waits for the next start because the dialog took the
/// focus from the game: when no token was sent, so the dialog showed, and
/// the token it gave was saved, so the next start goes ahead without it.
/// Without a saved token every start would show the dialog and none would
/// click, so the run goes ahead then. A token the portal rejected shows the
/// dialog too, but that is not told apart: portals may hand back a new token
/// on every start, so "a different token came back" would hold every time
/// on some, and the clicker would never start.
fn wait_for_next_start(sent: Option<&RestoreToken>, saved: bool) -> bool {
    sent.is_none() && saved
}

fn read_token(path: &Path) -> Option<RestoreToken> {
    token::read(path).unwrap_or_else(|err| {
        eprintln!(
            "auto-clicker: ignoring the portal token {}: {err}; the desktop asks again",
            path.display()
        );
        None
    })
}

/// Saves the token; `false`, after saying why, when it could not.
fn save_token(path: &Path, token: &RestoreToken) -> bool {
    match token::save(path, token) {
        Ok(()) => true,
        Err(err) => {
            eprintln!(
                "auto-clicker: could not save the portal token {}: {err}; the desktop asks again",
                path.display()
            );
            false
        }
    }
}

fn check_version(version: u32) -> Result<(), InputError> {
    if version >= MIN_VERSION {
        Ok(())
    } else {
        Err(InputError::PortalUnavailable(format!(
            "RemoteDesktop version {version} is too old; version {MIN_VERSION} is needed"
        )))
    }
}

/// Runs a portal call with [`CALL_TIMEOUT`].
async fn call<T>(what: &str, future: impl Future<Output = T>) -> Result<T, InputError> {
    timeout(CALL_TIMEOUT, future).await.map_err(|_| {
        InputError::Portal(format!(
            "{what}: no answer within {} s",
            CALL_TIMEOUT.as_secs()
        ))
    })
}

/// What a failed portal call means for the user.
fn portal_error(call: &str, err: ashpd::Error) -> InputError {
    match err {
        ashpd::Error::Response(ResponseError::Cancelled) => InputError::NotAllowed,
        ashpd::Error::Portal(PortalError::NotAllowed(why)) => InputError::PortalUnavailable(why),
        ashpd::Error::PortalNotFound(_) => {
            InputError::PortalUnavailable("the desktop has no RemoteDesktop portal".to_owned())
        }
        ashpd::Error::RequiresVersion(needed, found) => InputError::PortalUnavailable(format!(
            "RemoteDesktop version {found} is too old; version {needed} is needed"
        )),
        other => InputError::Portal(format!("{call}: {other}")),
    }
}

/// A portal session that closes when dropped, wherever that happens: a
/// failed step, a cancelled start, the end of a run.
pub(crate) struct SessionGuard(Option<Session<RemoteDesktop>>);

impl SessionGuard {
    /// No session: for tests that connect to a fake compositor directly.
    #[cfg(test)]
    pub(crate) fn none() -> SessionGuard {
        SessionGuard(None)
    }

    fn get(&self) -> Result<&Session<RemoteDesktop>, ashpd::Error> {
        // Only `drop` takes the session out.
        self.0.as_ref().ok_or(ashpd::Error::NoResponse)
    }
}

impl Drop for SessionGuard {
    /// Closes the session in a task of its own, bounded by
    /// [`CLOSE_TIMEOUT`]. Outside a runtime (the daemon is ending) there is
    /// nothing to run it on; closing the bus connection ends the session.
    fn drop(&mut self) {
        let Some(session) = self.0.take() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = timeout(CLOSE_TIMEOUT, session.close()).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_waits_for_the_next_start_only_after_a_first_dialog_whose_token_was_kept() {
        let token = RestoreToken::parse("t").unwrap();
        assert!(wait_for_next_start(None, true));
        assert!(!wait_for_next_start(Some(&token), true));
        assert!(!wait_for_next_start(Some(&token), false));
        // Without a saved token the next start would ask again: go ahead.
        assert!(!wait_for_next_start(None, false));
    }

    #[test]
    fn a_cancelled_dialog_is_not_allowed() {
        let err = portal_error("Start", ashpd::Error::Response(ResponseError::Cancelled));
        assert!(matches!(err, InputError::NotAllowed), "{err:?}");
    }

    #[test]
    fn missing_portal_pieces_are_unavailable_not_failures() {
        let no_app_id = portal_error(
            "CreateSession",
            ashpd::Error::Portal(PortalError::NotAllowed("An app id is required".to_owned())),
        );
        let old = portal_error("Start", ashpd::Error::RequiresVersion(2, 1));
        assert!(matches!(&no_app_id, InputError::PortalUnavailable(why) if why.contains("app id")));
        assert!(matches!(old, InputError::PortalUnavailable(_)));
        assert!(check_version(1).is_err());
        assert!(check_version(2).is_ok());
    }

    #[test]
    fn other_failures_name_the_call() {
        let err = portal_error("ConnectToEIS", ashpd::Error::NoResponse);
        assert!(matches!(&err, InputError::Portal(why) if why.starts_with("ConnectToEIS: ")));
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_that_does_not_answer_times_out() {
        let err = call("Start", std::future::pending::<()>())
            .await
            .unwrap_err();
        assert!(matches!(&err, InputError::Portal(why) if why.contains("no answer")));
    }
}
