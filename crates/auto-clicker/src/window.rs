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
//! The window's session ends when the window's end of the socket closes
//! (it exited, after every request it sent was read), when it sends a line
//! that does not parse or stops reading for [`WRITE_TIME`], and when the
//! controller ends it, and at once when the keeper ends. Each way drops the
//! window's process, which kills it if it still runs; tokio reaps it.

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
/// How long the requests of a window that stopped reading are still read.
const REAP_TIME: Duration = Duration::from_secs(2);
/// How many requests may wait for the controller.
const QUEUED_REQUESTS: usize = 4;

/// Starts the keeper of the settings window; each `()` sent to what it
/// returns is a choice of Settings in the tray. The keeper ends, and kills
/// the window, when every sender is gone.
pub fn keep(opener: WindowOpener) -> mpsc::UnboundedSender<()> {
    let program = program().map_err(|err| err.to_string());
    keep_with(Opening { opener, program })
}

/// [`keep`], opening windows as `opening` says.
fn keep_with(opening: Opening) -> mpsc::UnboundedSender<()> {
    let (open, asked) = mpsc::unbounded_channel();
    tokio::spawn(keeper(opening, asked));
    open
}

/// How a window opens: the program, or why it cannot be found, and where
/// its link to the controller goes.
struct Opening {
    opener: WindowOpener,
    program: Result<PathBuf, String>,
}

/// A window that runs: where to ask it to come to the front, and its
/// session, which kills it when dropped.
struct Running {
    raise: mpsc::UnboundedSender<()>,
    session: Aborting,
}

/// A task aborted when dropped: the session, wherever it waits (a write to
/// a window that stopped reading, or its last requests), and with it the
/// window, which is killed when its process is dropped.
struct Aborting(JoinHandle<()>);

impl Drop for Aborting {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn keeper(opening: Opening, mut asked: mpsc::UnboundedReceiver<()>) {
    let mut running: Option<Running> = None;
    loop {
        tokio::select! {
            choice = asked.recv() => match choice {
                Some(()) => running = chosen(running.take(), &opening),
                None => return,
            },
            () = finished(&mut running) => running = None,
        }
    }
}

/// Settings was chosen: the window that runs comes to the front, or a new
/// one opens.
fn chosen(running: Option<Running>, opening: &Opening) -> Option<Running> {
    match running {
        Some(window) if !window.session.0.is_finished() => {
            // The window may already be closing; then nothing comes to the
            // front, and the next choice opens one.
            let _ = window.raise.send(());
            Some(window)
        }
        _ => open(opening),
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
fn open(opening: &Opening) -> Option<Running> {
    let spawned = match &opening.program {
        Ok(program) => spawn(program),
        Err(why) => Err(io::Error::other(why.clone())),
    };
    match spawned {
        Ok((child, socket)) => {
            let (raise, raised) = mpsc::unbounded_channel();
            let opener = opening.opener.clone();
            let session = tokio::spawn(session(opener, child, socket, raised));
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
///
/// The window gets no `DISPLAY`: it is a Wayland window, its clipboard
/// speaks Wayland, and with an X display named, its toolkit's X11 code
/// could connect over TCP, which the program never does.
fn spawn(program: &std::path::Path) -> io::Result<(Child, UnixStream)> {
    let (ours, theirs) = std::os::unix::net::UnixStream::pair()?;
    let mut command = Command::new(program);
    command
        .stdin(Stdio::from(OwnedFd::from(theirs)))
        .kill_on_drop(true);
    command.env_remove("DISPLAY");
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
    child: Child,
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
        // Not on the window's exit: a window that sends a save and closes
        // at once has its end of the socket closed by then, and the
        // requests it sent before are read first, up to the end.
        let reading = read_requests(BufReader::new(read), requests);
        tokio::pin!(reading);
        let stopped = tokio::select! {
            () = &mut reading => None,
            stopped = write_updates(&mut write, &mut views, &mut raised) => Some(stopped),
        };
        // A window that stopped reading may have sent requests before it
        // closed: they are still read, up to the end of the socket.
        if stopped == Some(Stopped::WindowGone) {
            let _ = timeout(REAP_TIME, reading).await;
        }
    }
    // Spawned to be killed on drop: a window still running goes now.
    drop(child);
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

/// Why the updates stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stopped {
    /// The controller ended the session.
    ControllerGone,
    /// The window stopped reading, or closed its end.
    WindowGone,
}

/// Writes each view the controller publishes, and each request to come to
/// the front, until the controller ends the session or the window stops
/// reading; says which.
async fn write_updates(
    write: &mut (impl AsyncWrite + Unpin),
    views: &mut watch::Receiver<Option<auto_clicker_core::window::WindowView>>,
    raised: &mut mpsc::UnboundedReceiver<()>,
) -> Stopped {
    loop {
        let update = tokio::select! {
            changed = views.changed() => {
                if changed.is_err() {
                    return Stopped::ControllerGone;
                }
                match views.borrow_and_update().clone() {
                    Some(view) => Update::View(Box::new(view)),
                    None => continue,
                }
            }
            raise = raised.recv() => match raise {
                Some(()) => Update::Raise,
                None => return Stopped::ControllerGone,
            },
        };
        let Ok(bytes) = line(&update) else {
            return Stopped::ControllerGone;
        };
        if !matches!(
            timeout(WRITE_TIME, write.write_all(&bytes)).await,
            Ok(Ok(()))
        ) {
            return Stopped::WindowGone;
        }
    }
}

#[cfg(test)]
mod tests;
