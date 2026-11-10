//! One daemon per session: the first one holds a well-known name on the
//! session bus, and a second one finds it taken.

use futures_util::StreamExt;
use zbus::Connection;
use zbus::fdo::{DBusProxy, NameLostStream, RequestNameFlags, RequestNameReply};

/// The name the running daemon holds.
pub const BUS_NAME: &str = "io.github.tobylo.AutoClicker";

#[derive(Debug)]
pub enum ClaimError {
    /// Another daemon holds the name.
    Taken,
    /// The session bus is not reached through a Unix socket.
    Transport(String),
    /// The session bus could not be reached or answered with an error.
    Bus(zbus::Error),
}

/// The claimed name: the connection that holds it, and the signal that
/// says it was lost.
pub struct Claim {
    _bus: Connection,
    lost: NameLostStream,
}

/// Claims the daemon's name, on a session bus reached through a Unix socket
/// only: the program opens no network connection and runs no command, so a
/// `tcp:` or `unixexec:` address is refused rather than followed. The tray
/// connects through the same address, after this check.
pub async fn claim() -> Result<Claim, ClaimError> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
    check_address(address.as_deref()).map_err(ClaimError::Transport)?;
    let bus = Connection::session().await.map_err(ClaimError::Bus)?;
    // Subscribed before the name is requested, so its loss is never missed.
    let lost = DBusProxy::new(&bus)
        .await
        .map_err(ClaimError::Bus)?
        .receive_name_lost_with_args(&[(0, BUS_NAME)])
        .await
        .map_err(ClaimError::Bus)?;
    let reply = bus
        .request_name_with_flags(BUS_NAME, RequestNameFlags::DoNotQueue.into())
        .await;
    owned(reply)?;
    Ok(Claim { _bus: bus, lost })
}

/// Whether the name request made this daemon the owner. zbus reports a name
/// held by another as an `Exists` reply, and some paths as `NameTaken`.
fn owned(reply: zbus::Result<RequestNameReply>) -> Result<(), ClaimError> {
    match reply {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Ok(()),
        Ok(RequestNameReply::Exists | RequestNameReply::InQueue) | Err(zbus::Error::NameTaken) => {
            Err(ClaimError::Taken)
        }
        Err(err) => Err(ClaimError::Bus(err)),
    }
}

impl Claim {
    /// Resolves when the name is lost, or the session bus went away with it:
    /// another daemon could then claim it and open devices of its own.
    pub async fn lost(mut self) {
        // Either the signal or the end of the stream (the connection closed).
        let _ = self.lost.next().await;
    }
}

/// Accepts no address (zbus then uses the Unix socket in
/// `$XDG_RUNTIME_DIR`) or a list of Unix socket addresses only.
fn check_address(address: Option<&str>) -> Result<(), String> {
    let Some(address) = address else {
        return Ok(());
    };
    match address.split(';').find(|entry| !entry.starts_with("unix:")) {
        Some(entry) => Err(format!(
            "the session bus address {entry:?} is not a Unix socket"
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_held_by_another_is_taken() {
        assert!(owned(Ok(RequestNameReply::PrimaryOwner)).is_ok());
        assert!(owned(Ok(RequestNameReply::AlreadyOwner)).is_ok());
        for taken in [
            Ok(RequestNameReply::Exists),
            Ok(RequestNameReply::InQueue),
            Err(zbus::Error::NameTaken),
        ] {
            assert!(matches!(owned(taken), Err(ClaimError::Taken)));
        }
        let failed = owned(Err(zbus::Error::Failure("x".to_owned())));
        assert!(matches!(failed, Err(ClaimError::Bus(_))));
    }

    #[test]
    fn only_unix_socket_addresses_are_accepted() {
        assert!(check_address(None).is_ok());
        assert!(check_address(Some("unix:path=/run/user/1000/bus")).is_ok());
        assert!(check_address(Some("unix:path=/a;unix:abstract=b")).is_ok());
        for refused in [
            "tcp:host=example.com,port=1234",
            "unixexec:path=/tmp/payload",
            "unix:path=/a;tcp:host=b,port=1",
            "",
        ] {
            assert!(check_address(Some(refused)).is_err(), "{refused}");
        }
    }
}
