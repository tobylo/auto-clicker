//! One daemon per session: the first one holds a well-known name on the
//! session bus, and a second one finds it taken.

use zbus::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};

/// The name the running daemon holds.
pub const BUS_NAME: &str = "io.github.tobylo.AutoClicker";

#[derive(Debug)]
pub enum ClaimError {
    /// Another daemon holds the name.
    Taken,
    /// The session bus could not be reached or answered with an error.
    Bus(zbus::Error),
}

/// Claims the daemon's name. The name stays held while the returned
/// connection lives.
pub async fn claim() -> Result<Connection, ClaimError> {
    let bus = Connection::session().await.map_err(ClaimError::Bus)?;
    let reply = bus
        .request_name_with_flags(BUS_NAME, RequestNameFlags::DoNotQueue.into())
        .await;
    match reply {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Ok(bus),
        Ok(RequestNameReply::Exists | RequestNameReply::InQueue) | Err(zbus::Error::NameTaken) => {
            Err(ClaimError::Taken)
        }
        Err(err) => Err(ClaimError::Bus(err)),
    }
}
