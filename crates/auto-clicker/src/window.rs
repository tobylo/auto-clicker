//! The settings window: a program of its own, `auto-clicker-settings`
//! beside the daemon's binary, so its toolkit never loads into the daemon.
//!
//! Choosing Settings in the tray starts it, or asks the one running to come
//! to the front: there is one at a time. Each system starts it with a link
//! that nothing else can reach: one end of a socket pair as its standard
//! input on Linux, two pipes named on its command line on Windows. Over the
//! link the window sends requests, which go to the controller, and gets
//! every view the controller publishes for it (see
//! [`auto_clicker_core::window`]).
//!
//! The window's session ends when the window's end of the link closes (it
//! exited, after every request it sent was read), when it sends a line
//! that does not parse or stops reading for [`WRITE_TIME`], and when the
//! controller ends it, and at once when the keeper ends. Each way drops the
//! window's process, which kills it if it still runs.

use std::any::Any;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use auto_clicker_core::controller::{WindowLink, WindowOpener};
use auto_clicker_core::log;
use auto_clicker_core::window::{MAX_LINE, Request, Update, line, parse};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// The window's program, beside the daemon's.
#[cfg(not(windows))]
const PROGRAM: &str = "auto-clicker-settings";
#[cfg(windows)]
const PROGRAM: &str = "auto-clicker-settings.exe";
/// How long a write to the window may take before it counts as stuck.
const WRITE_TIME: Duration = Duration::from_secs(5);
/// How long the requests of a window that stopped reading are still read.
const REAP_TIME: Duration = Duration::from_secs(2);
/// How many requests may wait for the controller.
const QUEUED_REQUESTS: usize = 4;

/// A window that started: its process, killed when dropped, the link's
/// two ends, and what lets it come to the front when asked.
pub struct Started {
    pub process: Box<dyn Any + Send>,
    pub read: Box<dyn AsyncRead + Unpin + Send>,
    pub write: Box<dyn AsyncWrite + Unpin + Send>,
    /// Called before the window is asked to come to the front: Windows
    /// brings forward only a program the one in front lets.
    pub let_forward: Box<dyn Fn() + Send>,
}

/// How a system starts the window's program, with its link.
pub type Start = fn(&Path) -> io::Result<Started>;

/// Starts the keeper of the settings window, which starts windows with
/// `start`; each `()` sent to what it returns is a choice of Settings in
/// the tray. The keeper ends, and kills the window, when every sender is
/// gone.
pub fn keep(opener: WindowOpener, start: Start) -> mpsc::UnboundedSender<()> {
    let program = program().map_err(|err| err.to_string());
    keep_with(opener, program, start)
}

/// [`keep`], with `program` as the window's program, or why there is none.
pub(crate) fn keep_with(
    opener: WindowOpener,
    program: Result<PathBuf, String>,
    start: Start,
) -> mpsc::UnboundedSender<()> {
    let opening = Opening {
        opener,
        program,
        start,
    };
    let (open, asked) = mpsc::unbounded_channel();
    tokio::spawn(keeper(opening, asked));
    open
}

/// How a window opens: the program, or why it cannot be found, how it
/// starts, and where its link to the controller goes.
struct Opening {
    opener: WindowOpener,
    program: Result<PathBuf, String>,
    start: Start,
}

/// A window that runs: where to ask it to come to the front, and its
/// session, which kills it when dropped.
struct Running {
    raise: mpsc::UnboundedSender<()>,
    let_forward: Box<dyn Fn() + Send>,
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
            (window.let_forward)();
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
    let started = match &opening.program {
        Ok(program) => (opening.start)(program),
        Err(why) => Err(io::Error::other(why.clone())),
    };
    match started {
        Ok(Started {
            process,
            read,
            write,
            let_forward,
        }) => {
            let (raise, raised) = mpsc::unbounded_channel();
            let opener = opening.opener.clone();
            let session = tokio::spawn(session(opener, process, (read, write), raised));
            Some(Running {
                raise,
                let_forward,
                session: Aborting(session),
            })
        }
        Err(err) => {
            log::line(format_args!(
                "auto-clicker: could not open the settings window: {err}"
            ));
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

/// The two ends of a window's link.
type Ends = (
    Box<dyn AsyncRead + Unpin + Send>,
    Box<dyn AsyncWrite + Unpin + Send>,
);

/// One window's session: links it to the controller and carries requests
/// and updates until either side ends it, then makes sure the window is
/// gone.
async fn session(
    opener: WindowOpener,
    process: Box<dyn Any + Send>,
    (read, mut write): Ends,
    mut raised: mpsc::UnboundedReceiver<()>,
) {
    let (requests, received) = mpsc::channel(QUEUED_REQUESTS);
    let (view, mut views) = watch::channel(None);
    let link = WindowLink {
        requests: received,
        view,
    };
    if opener.send(link).is_ok() {
        // Not on the window's exit: a window that sends a save and closes
        // at once has its end of the link closed by then, and the requests
        // it sent before are read first, up to the end.
        let reading = read_requests(BufReader::new(read), requests);
        tokio::pin!(reading);
        let stopped = tokio::select! {
            () = &mut reading => None,
            stopped = write_updates(&mut write, &mut views, &mut raised) => Some(stopped),
        };
        // A window that stopped reading may have sent requests before it
        // closed: they are still read, up to the end of the link.
        if stopped == Some(Stopped::WindowGone) {
            let _ = timeout(REAP_TIME, reading).await;
        }
    }
    // Started to be killed on drop: a window still running goes now.
    drop(process);
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
pub(crate) mod tests;
