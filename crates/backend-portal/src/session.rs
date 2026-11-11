//! The RemoteDesktop portal: the app id, the permission and the EIS socket.
//! The registered connection also serves the GlobalShortcuts portal
//! ([`crate::shortcuts`]).
//!
//! The portal scopes its stored grant by the caller's app id, and a host
//! program gets one only by calling `Registry.Register` before any other
//! portal call on its connection (X02). [`Portal::connect`] therefore opens
//! a connection of its own and registers on it first; nothing else uses a
//! connection that is not registered.
//!
//! Each open gets a new connection, owned by the open and then by the run.
//! Dropping it ends the session: the portal closes every session of a
//! client that leaves the bus. So a cancelled open, a failed step and the
//! end of a run all close the session without a `Close` call that could
//! fail or hang, and a portal service that restarted meets a connection
//! that registers again.

use std::cell::RefCell;
use std::future::Future;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ashpd::desktop::remote_desktop::{
    ConnectToEISOptions, DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, ResponseError, Session};
use ashpd::enumflags2::BitFlags;
use ashpd::{AppID, PortalError};
use auto_clicker_core::input::InputError;
use tokio::time::timeout;

use crate::answer::{refused, version};
use crate::token::{self, RestoreToken};

/// The interface clicks go through.
const REMOTE_DESKTOP: &str = "org.freedesktop.portal.RemoteDesktop";

/// The program's app id: the desktop file the package installs.
pub const APP_ID: &str = "io.github.tobylo.AutoClicker";

/// How long a portal call may take, apart from the dialog.
pub(crate) const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the permission dialog may stay open before the start gives up.
pub(crate) const DIALOG_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// A `Start` that took this long waited on a person: the dialog showed.
/// Restoring from a token takes 7 to 70 ms (X02, P01).
const SLOW_START: Duration = Duration::from_secs(1);
/// The first RemoteDesktop version with `ConnectToEIS`.
const MIN_VERSION: u32 = 2;

/// What the program asks for.
fn wanted() -> BitFlags<DeviceType> {
    DeviceType::Keyboard | DeviceType::Pointer
}

/// A session-bus connection registered under [`APP_ID`].
#[derive(Debug)]
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
        // Every method call on it gets the call deadline, zbus's own cleanup
        // included (the `RemoveMatch` calls a dropped stream sends), so
        // nothing holds the connection, and with it the session, for long
        // after it was dropped. The dialog's answer is a signal, not a
        // method reply, so it is not cut short.
        let connection = call("session bus", async {
            zbus::connection::Builder::session()?
                .method_timeout(CALL_TIMEOUT)
                .build()
                .await
        })
        .await?
        .map_err(|err| InputError::Portal(format!("session bus: {err}")))?;
        let app_id = AppID::try_from(APP_ID)
            .map_err(|err| InputError::PortalUnavailable(format!("app id {APP_ID}: {err}")))?;
        call(
            "Registry.Register",
            ashpd::register_host_app_with_connection(connection.clone(), app_id),
        )
        .await?
        .map_err(registration_failed)?;
        Ok(Portal { connection })
    }

    /// The registered connection, for another portal interface. A session
    /// made on it ends when the connection is dropped.
    pub(crate) fn connection(&self) -> &zbus::Connection {
        &self.connection
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
        // Read again without ashpd, which takes a failed read for version 1.
        check_version(version(&self.connection, REMOTE_DESKTOP).await?)?;
        Ok(proxy)
    }

    /// Starts a session for a keyboard and a pointer, restoring the grant
    /// with the token from `tokens` and keeping the token the portal hands
    /// back. Fails with [`InputError::AllowedNow`] when the dialog showed:
    /// the run waits for the next start.
    ///
    /// The session lives on this connection: dropping the future part way,
    /// or the result, closes it.
    pub(crate) async fn start(self, tokens: &RefCell<Tokens>) -> Result<Started, InputError> {
        let proxy = self.remote_desktop().await?;
        let session = call(
            "CreateSession",
            proxy.create_session(CreateSessionOptions::default()),
        )
        .await?
        .map_err(|err| portal_error("CreateSession", err))?;
        let sent = tokens.borrow_mut().token_to_send();
        let options = SelectDevicesOptions::default()
            .set_devices(wanted())
            .set_persist_mode(PersistMode::ExplicitlyRevoked)
            .set_restore_token(sent.as_ref().map(RestoreToken::as_str));
        call("SelectDevices", async {
            proxy.select_devices(&session, options).await?.response()
        })
        .await?
        .map_err(|err| portal_error("SelectDevices", err))?;
        let asked = Instant::now();
        let started = timeout(DIALOG_TIMEOUT, async {
            proxy
                .start(&session, None, StartOptions::default())
                .await?
                .response()
        })
        .await
        .map_err(|_| {
            InputError::Portal("the permission dialog was left open for 5 min".to_owned())
        })?
        .map_err(|err| portal_error("Start", err))?;
        let returned = started.restore_token().and_then(RestoreToken::parse);
        if tokens
            .borrow_mut()
            .started(sent.is_some(), asked.elapsed(), returned)
        {
            return Err(InputError::AllowedNow);
        }
        let socket = call(
            "ConnectToEIS",
            proxy.connect_to_eis(&session, ConnectToEISOptions::default()),
        )
        .await?
        .map_err(|err| portal_error("ConnectToEIS", err))?;
        Ok(Started {
            session: SessionGuard {
                _held: Some((session, self.connection)),
            },
            socket,
            devices: started.devices(),
        })
    }
}

/// The restore tokens: the file, and the last token the portal handed out,
/// which is kept even when saving it failed, so a start after a failed save
/// does not show the dialog again.
#[derive(Debug, Default)]
pub(crate) struct Tokens {
    path: Option<PathBuf>,
    last: Option<RestoreToken>,
    /// The last start showed the dialog, so the run waited for this one.
    just_allowed: bool,
}

impl Tokens {
    pub(crate) fn new(path: Option<PathBuf>) -> Tokens {
        Tokens {
            path,
            last: None,
            just_allowed: false,
        }
    }

    /// The token to restore with: the last one handed out, else the file's.
    fn token_to_send(&mut self) -> Option<RestoreToken> {
        self.last
            .clone()
            .or_else(|| self.path.as_deref().and_then(read_token))
    }

    /// Notes a started session, keeping and saving the token it returned;
    /// `true` when the run waits for the next start.
    fn started(&mut self, sent: bool, took: Duration, returned: Option<RestoreToken>) -> bool {
        if let Some(token) = returned {
            if let Some(path) = &self.path {
                save_token(path, &token);
            }
            self.last = Some(token);
        }
        let wait = wait_for_next_start(sent, took, self.just_allowed);
        self.just_allowed = wait;
        wait
    }
}

/// Whether the run waits for the next start because the dialog took the
/// focus from the game. The dialog showed when no token was sent, or when
/// `Start` waited on a person (a token the portal rejected). The returned
/// token says nothing either way: portals may hand back a new one on every
/// start. Never twice in a row: the start after a dialog goes ahead, so a
/// portal that keeps no tokens, or a slow one, cannot keep the clicker from
/// ever starting.
fn wait_for_next_start(sent: bool, took: Duration, just_allowed: bool) -> bool {
    !just_allowed && (!sent || took >= SLOW_START)
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

fn save_token(path: &Path, token: &RestoreToken) {
    if let Err(err) = token::save(path, token) {
        eprintln!(
            "auto-clicker: could not save the portal token {}: {err}; the desktop asks again \
             after a restart",
            path.display()
        );
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

/// Why `Registry.Register` failed: the portal's answer that it cannot
/// register the program, most often for want of the desktop file, or a
/// call that got no answer and may work later.
fn registration_failed(err: ashpd::Error) -> InputError {
    let refusal = |why: &dyn std::fmt::Display| {
        InputError::PortalUnavailable(format!(
            "could not register the app id {APP_ID} ({why}); is the package's {APP_ID}.desktop \
             file installed?"
        ))
    };
    match err {
        ashpd::Error::Zbus(err) => match zbus::fdo::Error::from(err) {
            err if refused(&err) => refusal(&err),
            err => InputError::Portal(format!("Registry.Register: {err}")),
        },
        ashpd::Error::PortalNotFound(_) => refusal(&"the desktop has no portal registry"),
        other => InputError::Portal(format!("Registry.Register: {other}")),
    }
}

/// Runs a portal call with [`CALL_TIMEOUT`].
pub(crate) async fn call<T>(what: &str, future: impl Future<Output = T>) -> Result<T, InputError> {
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

/// A started session and the connection it lives on. Dropping it closes
/// the connection, which ends the session.
pub(crate) struct SessionGuard {
    _held: Option<(Session<RemoteDesktop>, zbus::Connection)>,
}

impl SessionGuard {
    /// No session: for tests that connect to a fake compositor directly.
    #[cfg(test)]
    pub(crate) fn none() -> SessionGuard {
        SessionGuard { _held: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: Duration = Duration::from_millis(70);
    const SLOW: Duration = Duration::from_secs(3);

    #[test]
    fn a_run_waits_for_the_next_start_after_a_dialog_but_never_twice_in_a_row() {
        // The first start: no token, so the dialog showed.
        assert!(wait_for_next_start(false, FAST, false));
        // A restore from a token goes ahead.
        assert!(!wait_for_next_start(true, FAST, false));
        // A rejected token: the dialog showed, and Start waited on it.
        assert!(wait_for_next_start(true, SLOW, false));
        // Right after a dialog, the next start goes ahead whatever happens.
        assert!(!wait_for_next_start(false, SLOW, true));
        assert!(!wait_for_next_start(true, SLOW, true));
        assert!(wait_for_next_start(true, SLOW_START, false));
    }

    fn token(text: &str) -> Option<RestoreToken> {
        RestoreToken::parse(text)
    }

    #[test]
    fn a_token_that_could_not_be_saved_is_still_used() {
        // A path whose directory cannot be created.
        let mut tokens = Tokens::new(Some(PathBuf::from("/proc/auto-clicker/portal-token")));
        assert_eq!(tokens.token_to_send(), None);
        assert!(tokens.started(false, FAST, token("granted")));
        assert_eq!(tokens.token_to_send(), token("granted"));
        assert!(!tokens.started(true, FAST, token("granted")));
    }

    #[test]
    fn without_a_token_path_the_portal_never_locks_the_clicker_out() {
        // No token path and a portal that returns none: every start asks,
        // but only every other one waits.
        let mut tokens = Tokens::new(None);
        let waits: Vec<bool> = (0..4).map(|_| tokens.started(false, SLOW, None)).collect();
        assert_eq!(waits, [true, false, true, false]);
    }

    #[test]
    fn the_last_token_handed_out_wins_over_the_file() {
        let dir = std::env::temp_dir().join(format!(
            "auto-clicker-tokens-{}-{:016x}",
            std::process::id(),
            fastrand::u64(..)
        ));
        let path = dir.join("portal-token");
        let mut tokens = Tokens::new(Some(path.clone()));
        tokens.started(true, FAST, token("first"));
        assert_eq!(token::read(&path).unwrap(), token("first"));
        // The file changes under the daemon: the token in memory is newer.
        token::save(&path, &RestoreToken::parse("other").unwrap()).unwrap();
        assert_eq!(tokens.token_to_send(), token("first"));
        // A fresh daemon reads the file.
        assert_eq!(Tokens::new(Some(path)).token_to_send(), token("other"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_cancelled_dialog_is_not_allowed() {
        let err = portal_error("Start", ashpd::Error::Response(ResponseError::Cancelled));
        assert!(matches!(err, InputError::NotAllowed), "{err:?}");
    }

    #[test]
    fn a_refused_registration_is_unavailable_and_a_lost_one_is_not() {
        let fdo = |err| ashpd::Error::Zbus(zbus::Error::FDO(Box::new(err)));
        let failed = registration_failed(fdo(zbus::fdo::Error::Failed(
            "App info not found".to_owned(),
        )));
        assert!(
            matches!(&failed, InputError::PortalUnavailable(why) if why.contains(".desktop")),
            "{failed:?}"
        );
        let lost = registration_failed(fdo(zbus::fdo::Error::NoReply("gone".to_owned())));
        assert!(matches!(lost, InputError::Portal(_)), "{lost:?}");
        let cut = registration_failed(ashpd::Error::Zbus(zbus::Error::InterfaceNotFound));
        assert!(matches!(cut, InputError::Portal(_)), "{cut:?}");
        let name =
            zbus::names::OwnedInterfaceName::try_from("org.freedesktop.host.portal.Registry")
                .unwrap();
        let absent = registration_failed(ashpd::Error::PortalNotFound(name));
        assert!(matches!(absent, InputError::PortalUnavailable(_)));
        let io = registration_failed(ashpd::Error::NoResponse);
        assert!(matches!(io, InputError::Portal(_)));
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
