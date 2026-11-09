//! A RemoteDesktop portal session, its restore token, and its EIS socket.

use std::os::fd::OwnedFd;
use std::path::Path;
use std::time::Instant;

use ashpd::desktop::remote_desktop::{
    ConnectToEISOptions, DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, Session};

use crate::log::line;

/// An open RemoteDesktop session. Dropping it leaves the session open until
/// the D-Bus connection closes; [`Remote::close`] ends it explicitly.
pub struct Remote {
    proxy: RemoteDesktop,
    session: Session<RemoteDesktop>,
}

impl Remote {
    /// Creates a session for a keyboard and a pointer, restoring the
    /// permission from `token_file` when it holds a token, and saves the new
    /// token there. Returns the session and its EIS socket.
    pub async fn open(token_file: &Path) -> Result<(Remote, OwnedFd), String> {
        let proxy = RemoteDesktop::new().await.map_err(dbus("RemoteDesktop"))?;
        line(&format!("RemoteDesktop version {}", proxy.version()));
        let session = proxy
            .create_session(CreateSessionOptions::default())
            .await
            .map_err(dbus("CreateSession"))?;
        let remote = Remote { proxy, session };
        let old_token = read_token(token_file);
        remote.select(old_token.as_deref()).await?;
        let new_token = remote.start().await?;
        report_token(old_token.as_deref(), new_token.as_deref());
        save_token(token_file, new_token.as_deref())?;
        let fd = remote.connect().await?;
        Ok((remote, fd))
    }

    async fn select(&self, token: Option<&str>) -> Result<(), String> {
        let options = SelectDevicesOptions::default()
            .set_devices(DeviceType::KEYBOARD | DeviceType::POINTER)
            .set_persist_mode(PersistMode::ExplicitlyRevoked)
            .set_restore_token(token);
        self.proxy
            .select_devices(&self.session, options)
            .await
            .map_err(dbus("SelectDevices"))?
            .response()
            .map_err(dbus("SelectDevices response"))
    }

    /// Starts the session; this is where the permission dialog appears.
    async fn start(&self) -> Result<Option<String>, String> {
        let started = Instant::now();
        let response = self
            .proxy
            .start(&self.session, None, StartOptions::default())
            .await
            .map_err(dbus("Start"))?
            .response()
            .map_err(dbus("Start response"))?;
        line(&format!(
            "Start answered after {} ms: devices {:?}",
            started.elapsed().as_millis(),
            response.devices()
        ));
        Ok(response.restore_token().map(str::to_owned))
    }

    async fn connect(&self) -> Result<OwnedFd, String> {
        self.proxy
            .connect_to_eis(&self.session, ConnectToEISOptions::default())
            .await
            .map_err(dbus("ConnectToEIS"))
    }

    /// Closes the portal session; the compositor then ends its EIS client.
    pub async fn close(&self) -> Result<(), String> {
        self.session.close().await.map_err(dbus("Session.Close"))
    }
}

fn dbus(call: &'static str) -> impl Fn(ashpd::Error) -> String {
    move |err| format!("{call}: {err}")
}

fn read_token(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let token = text.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

fn report_token(old: Option<&str>, new: Option<&str>) {
    let verdict = match (old, new) {
        (None, Some(_)) => "first token issued",
        (Some(_), Some(new_token)) if Some(new_token) == old => "same token returned",
        (Some(_), Some(_)) => "new token issued (old one replaced)",
        (_, None) => "no token returned",
    };
    line(&format!(
        "restore token: sent {}, {verdict}",
        if old.is_some() { "one" } else { "none" }
    ));
}

/// The token from the latest Start replaces the one saved before it.
fn save_token(path: &Path, token: Option<&str>) -> Result<(), String> {
    match token {
        Some(token) => std::fs::write(path, token),
        None => match std::fs::remove_file(path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        },
    }
    .map_err(|err| format!("token file {}: {err}", path.display()))
}
