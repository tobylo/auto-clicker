//! One ask over Hyprland's socket: a new connection, `cursorpos`, the
//! answer read to its end (W01 brief, W6).
//!
//! Each step has its own deadline ([`DEADLINE`]): the connect, then the
//! request and its answer to the end, so an ask takes at most twice that.
//! A deadline per step, not one for the whole ask, so a moment the daemon's
//! own thread was busy before the connect was seen through (opening the
//! mice when a recording starts: 78 ms measured) is not taken for a late
//! answer; an answer read only after its deadline is late, however early
//! it came. At most [`LIMIT`] bytes are read. The connection's peer is
//! checked before anything is written: its process must be the one the
//! socket was found to belong to, so nothing is ever asked of another
//! display's Hyprland. A failure or a late answer is no position; dropping
//! the ask closes its connection.

use std::path::Path;
use std::time::Duration;

use auto_clicker_core::message::NoPointer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::{Instant, timeout, timeout_at};

use crate::reply::{Global, parse};

/// How long each step of an ask may take, the connect and then the answer:
/// well past the longest ask measured (3.1 ms under a game's load, decision
/// 0004), well inside a modeset, during which Hyprland holds its answers for
/// seconds.
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
/// passes `belongs`: the connect within [`DEADLINE`], then the answer
/// within [`DEADLINE`] of the request.
pub(crate) async fn ask(path: &Path, belongs: impl FnOnce(u32) -> bool) -> Result<Global, Failure> {
    let stream = match timeout(DEADLINE, UnixStream::connect(path)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(_)) | Err(_) => return Err(Failure::NoAnswer),
    };
    // Checked again once a position is read: one read only after the
    // daemon's thread was busy past the deadline is late, though it came
    // in time, since it is stamped when read. A refusal stays a refusal.
    let deadline = Instant::now() + DEADLINE;
    match timeout_at(deadline, exchange(stream, belongs)).await {
        Ok(Ok(_)) if Instant::now() > deadline => Err(Failure::NoAnswer),
        Ok(answer) => answer,
        Err(_) => Err(Failure::NoAnswer),
    }
}

async fn exchange(
    mut stream: UnixStream,
    belongs: impl FnOnce(u32) -> bool,
) -> Result<Global, Failure> {
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
    // One byte past the limit is enough to tell an answer too long: no
    // answer that long parses.
    (&mut stream)
        .take(limit.saturating_add(1))
        .read_to_end(&mut answer)
        .await
        .map_err(|_| Failure::NoAnswer)?;
    parse(&answer).ok_or(Failure::Unreadable)
}

#[cfg(test)]
mod tests;
