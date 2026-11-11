//! The settings window: a program of its own, `auto-clicker-settings`
//! beside the daemon's binary, so its toolkit never loads into the daemon.
//!
//! Choosing Settings in the tray starts it, or asks the one running to come
//! to the front: there is one at a time. The daemon gives it one end of a
//! socket pair as its standard input, and nothing else can reach it. Over
//! the socket the window sends requests, which go to the controller, and
//! gets every view the controller publishes for it (see
//! [`auto_clicker_core::window`]).
//!
//! The window's session ends when the window closes its end, sends a line
//! that does not parse, stops reading for [`WRITE_TIME`], or exits, and
//! when the controller ends it. Each way kills the window if it still runs,
//! and waits [`REAP_TIME`] at most for it to go.

use std::io;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use auto_clicker_core::controller::{WindowLink, WindowOpener};
use auto_clicker_core::window::{MAX_LINE, Request, Update, line, parse};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// The window's program, beside the daemon's.
const PROGRAM: &str = "auto-clicker-settings";
/// How long a write to the window may take before it counts as stuck.
const WRITE_TIME: Duration = Duration::from_secs(5);
/// How long the daemon waits for a window it killed to go.
const REAP_TIME: Duration = Duration::from_secs(2);
/// How many requests may wait for the controller.
const QUEUED_REQUESTS: usize = 4;

/// Starts the keeper of the settings window; each `()` sent to what it
/// returns is a choice of Settings in the tray. The keeper ends, and kills
/// the window, when every sender is gone.
pub fn keep(opener: WindowOpener) -> mpsc::UnboundedSender<()> {
    let (open, asked) = mpsc::unbounded_channel();
    tokio::spawn(keeper(opener, asked));
    open
}

/// A window that runs: where to ask it to come to the front, and its
/// session, which kills it when dropped.
struct Running {
    raise: mpsc::UnboundedSender<()>,
    session: Aborting,
}

/// A task aborted when dropped: the session, and with it the window.
struct Aborting(JoinHandle<()>);

impl Drop for Aborting {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn keeper(opener: WindowOpener, mut asked: mpsc::UnboundedReceiver<()>) {
    let mut running: Option<Running> = None;
    loop {
        tokio::select! {
            choice = asked.recv() => match choice {
                Some(()) => running = chosen(running.take(), &opener),
                None => return,
            },
            () = finished(&mut running) => running = None,
        }
    }
}

/// Settings was chosen: the window that runs comes to the front, or a new
/// one opens.
fn chosen(running: Option<Running>, opener: &WindowOpener) -> Option<Running> {
    match running {
        Some(window) if !window.session.0.is_finished() => {
            // The window may already be closing; then nothing comes to the
            // front, and the next choice opens one.
            let _ = window.raise.send(());
            Some(window)
        }
        _ => open(opener),
    }
}

/// When the running window's session ends; never while none runs.
async fn finished(running: &mut Option<Running>) {
    match running {
        Some(window) => {
            // An aborted or panicked session ended just the same.
            let _ = (&mut window.session.0).await;
        }
        None => std::future::pending().await,
    }
}

/// Starts a window and its session; `None`, saying why, when it cannot.
fn open(opener: &WindowOpener) -> Option<Running> {
    match spawn() {
        Ok((child, socket)) => {
            let (raise, raised) = mpsc::unbounded_channel();
            let session = tokio::spawn(session(opener.clone(), child, socket, raised));
            Some(Running {
                raise,
                session: Aborting(session),
            })
        }
        Err(err) => {
            eprintln!("auto-clicker: could not open the settings window: {err}");
            None
        }
    }
}

/// The window's program: beside the daemon's own.
fn program() -> io::Result<PathBuf> {
    let daemon = std::env::current_exe()?;
    let dir = daemon
        .parent()
        .ok_or_else(|| io::Error::other("the daemon's binary is in no directory"))?;
    Ok(dir.join(PROGRAM))
}

/// Starts the window with one end of a new socket pair as its standard
/// input, and returns it with the other end. Both ends close on exec: the
/// window gets its end as file descriptor 0 alone, and no other program the
/// daemon might start gets either.
fn spawn() -> io::Result<(Child, UnixStream)> {
    let (ours, theirs) = std::os::unix::net::UnixStream::pair()?;
    let mut command = Command::new(program()?);
    command
        .stdin(Stdio::from(OwnedFd::from(theirs)))
        .kill_on_drop(true);
    let child = command.spawn()?;
    // The command keeps a copy of the window's end until dropped; with it
    // gone, the window's end closes when the window does.
    drop(command);
    ours.set_nonblocking(true)?;
    Ok((child, UnixStream::from_std(ours)?))
}

/// One window's session: links it to the controller and carries requests
/// and updates until either side ends it, then makes sure the window is
/// gone.
async fn session(
    opener: WindowOpener,
    mut child: Child,
    socket: UnixStream,
    mut raised: mpsc::UnboundedReceiver<()>,
) {
    let (requests, received) = mpsc::channel(QUEUED_REQUESTS);
    let (view, mut views) = watch::channel(None);
    let link = WindowLink {
        requests: received,
        view,
    };
    if opener.send(link).is_ok() {
        let (read, mut write) = socket.into_split();
        tokio::select! {
            () = read_requests(BufReader::new(read), requests) => {}
            () = write_updates(&mut write, &mut views, &mut raised) => {}
            _ = child.wait() => {}
        }
    }
    end(&mut child).await;
}

/// Kills the window if it still runs, and waits a little for it to go.
async fn end(child: &mut Child) {
    // It may have exited already; then there is nothing to kill.
    let _ = child.start_kill();
    let _ = timeout(REAP_TIME, child.wait()).await;
}

/// Hands each request the window sends to the controller, until the window
/// closes its end, sends a line that is no request, or the controller goes.
async fn read_requests(mut read: impl AsyncBufRead + Unpin, requests: mpsc::Sender<Request>) {
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        let limit = u64::try_from(MAX_LINE).unwrap_or(u64::MAX);
        match (&mut read).take(limit).read_until(b'\n', &mut bytes).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        // A line cut short, by the limit or by the end, is no line.
        if bytes.pop() != Some(b'\n') {
            return;
        }
        let Ok(request) = parse::<Request>(&bytes) else {
            return;
        };
        if requests.send(request).await.is_err() {
            return;
        }
    }
}

/// Writes each view the controller publishes, and each request to come to
/// the front, until the controller ends the session or the window stops
/// reading.
async fn write_updates(
    write: &mut (impl AsyncWrite + Unpin),
    views: &mut watch::Receiver<Option<auto_clicker_core::window::WindowView>>,
    raised: &mut mpsc::UnboundedReceiver<()>,
) {
    loop {
        let update = tokio::select! {
            changed = views.changed() => {
                if changed.is_err() {
                    return;
                }
                match views.borrow_and_update().clone() {
                    Some(view) => Update::View(Box::new(view)),
                    None => continue,
                }
            }
            raise = raised.recv() => match raise {
                Some(()) => Update::Raise,
                None => return,
            },
        };
        let Ok(bytes) = line(&update) else {
            return;
        };
        if !matches!(
            timeout(WRITE_TIME, write.write_all(&bytes)).await,
            Ok(Ok(()))
        ) {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
