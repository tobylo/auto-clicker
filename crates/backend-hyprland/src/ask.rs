//! One ask over Hyprland's socket: a new connection, `cursorpos`, the
//! answer read to its end (W01 brief, W6).
//!
//! Each ask has its own deadline ([`DEADLINE`]) from the connect to the
//! answer's end, and reads at most [`LIMIT`] bytes. The connection's peer is
//! checked before anything is written: its process must be the one the
//! socket was found to belong to, so nothing is ever asked of another
//! display's Hyprland. A failure or a late answer is no position; dropping
//! the ask closes its connection.

use std::path::Path;
use std::time::Duration;

use auto_clicker_core::message::NoPointer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

use crate::reply::{Global, parse};

/// How long one ask may take, connect to answer: well past the longest
/// measured (3.1 ms under a game's load, decision 0004), well inside a
/// modeset, during which Hyprland holds its answers for seconds.
pub(crate) const DEADLINE: Duration = Duration::from_millis(50);

/// The most an answer may hold. The longest Hyprland gives is 24 bytes.
pub(crate) const LIMIT: usize = 64;

/// Why an ask gave no position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// No connection, or no whole answer before the deadline.
    NoAnswer,
    /// An answer, but not in Hyprland's form.
    Unreadable,
    /// The socket's process is not the one asked of.
    OtherProcess,
}

impl Failure {
    /// What the tray says while asks fail so.
    pub(crate) fn reason(self) -> NoPointer {
        match self {
            Failure::NoAnswer => NoPointer::HyprlandNoAnswer,
            Failure::Unreadable => NoPointer::HyprlandUnreadable,
            Failure::OtherProcess => NoPointer::HyprlandOtherDisplay,
        }
    }
}

/// Asks the socket at `path` where the pointer is, once its peer's process
/// passes `belongs`, within [`DEADLINE`].
pub(crate) async fn ask(path: &Path, belongs: impl FnOnce(u32) -> bool) -> Result<Global, Failure> {
    match timeout(DEADLINE, exchange(path, belongs)).await {
        Ok(answer) => answer,
        Err(_) => Err(Failure::NoAnswer),
    }
}

async fn exchange(path: &Path, belongs: impl FnOnce(u32) -> bool) -> Result<Global, Failure> {
    let mut stream = UnixStream::connect(path)
        .await
        .map_err(|_| Failure::NoAnswer)?;
    let peer = stream
        .peer_cred()
        .ok()
        .and_then(|credentials| credentials.pid())
        .and_then(|pid| u32::try_from(pid).ok());
    if !peer.is_some_and(belongs) {
        return Err(Failure::OtherProcess);
    }
    stream
        .write_all(b"cursorpos")
        .await
        .map_err(|_| Failure::NoAnswer)?;
    let mut answer = Vec::with_capacity(LIMIT);
    let limit = u64::try_from(LIMIT).unwrap_or(u64::MAX);
    (&mut stream)
        .take(limit.saturating_add(1))
        .read_to_end(&mut answer)
        .await
        .map_err(|_| Failure::NoAnswer)?;
    if answer.len() > LIMIT {
        return Err(Failure::Unreadable);
    }
    parse(&answer).ok_or(Failure::Unreadable)
}

#[cfg(test)]
mod tests;
