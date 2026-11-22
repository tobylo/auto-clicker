//! One wait: the Wayland connection, a line on stdin, or a deadline. No
//! polling loop: the process sleeps in `poll` until one of them.

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use wayland_client::backend::WaylandError;
use wayland_client::{DispatchError, EventQueue};

use crate::say;

/// Commands read from stdin, split into lines.
pub struct Input {
    open: bool,
    pending: Vec<u8>,
}

impl Input {
    pub fn new() -> Input {
        Input {
            open: true,
            pending: Vec::new(),
        }
    }

    /// Whether stdin has ended; the probe then quits.
    pub fn closed(&self) -> bool {
        !self.open
    }

    fn read(&mut self) -> Vec<String> {
        let mut chunk = [0u8; 4096];
        match rustix::io::read(std::io::stdin(), &mut chunk) {
            Ok(0) | Err(_) => self.open = false,
            Ok(count) => self.pending.extend_from_slice(&chunk[..count]),
        }
        let mut lines = Vec::new();
        while let Some(end) = self.pending.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            lines.push(String::from_utf8_lossy(&line).trim().to_owned());
        }
        lines
    }
}

/// Why the connection ended.
pub fn describe(error: &DispatchError) -> String {
    format!("{error}")
}

/// Dispatches what has come, then sleeps until the connection, stdin or
/// `deadline` (seconds on `CLOCK_MONOTONIC`) and dispatches again. Returns
/// the lines read; an error is the connection's end.
pub fn wait<S>(
    queue: &mut EventQueue<S>,
    state: &mut S,
    input: &mut Input,
    deadline: Option<f64>,
) -> Result<Vec<String>, String> {
    queue
        .dispatch_pending(state)
        .map_err(|error| describe(&error))?;
    queue.flush().map_err(|error| format!("{error}"))?;
    let Some(guard) = queue.prepare_read() else {
        return Ok(Vec::new());
    };
    let timeout = deadline.map(|at| timespec((at - say::now()).max(0.0)));
    let stdin = std::io::stdin();
    let (wayland, typed) = {
        let connection = guard.connection_fd();
        let mut fds = vec![PollFd::new(&connection, PollFlags::IN)];
        if input.open {
            fds.push(PollFd::new(&stdin, PollFlags::IN));
        }
        // EINTR is a wake like any other: the caller checks its deadline.
        let _ = poll(&mut fds, timeout.as_ref());
        let ready = |fd: &PollFd<'_>| !fd.revents().is_empty();
        (ready(&fds[0]), fds.get(1).is_some_and(ready))
    };
    if wayland {
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(format!("{error}")),
        }
    } else {
        drop(guard);
    }
    queue
        .dispatch_pending(state)
        .map_err(|error| describe(&error))?;
    Ok(if typed { input.read() } else { Vec::new() })
}

fn timespec(seconds: f64) -> Timespec {
    let whole = seconds.floor();
    Timespec {
        tv_sec: whole as i64,
        tv_nsec: ((seconds - whole) * 1e9) as i64,
    }
}
