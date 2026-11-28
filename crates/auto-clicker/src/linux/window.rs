//! How the settings window starts on Linux: with one end of a socket pair
//! as its standard input. The keeper is the daemon's own ([`crate::window`]).

use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::process::Stdio;

use auto_clicker_core::controller::WindowOpener;
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::window::{self, Started};

/// Starts the keeper of the settings window; each `()` sent to what it
/// returns is a choice of Settings in the tray.
pub fn keep(opener: WindowOpener) -> mpsc::UnboundedSender<()> {
    window::keep(opener, start)
}

/// Starts the window with one end of a new socket pair as its standard
/// input, and returns it with the other end. Both ends close on exec: the
/// window gets its end as file descriptor 0 alone, and no other program the
/// daemon might start gets either. tokio reaps the window once it is
/// killed.
///
/// The window gets no `DISPLAY`: it is a Wayland window, its clipboard
/// speaks Wayland, and with an X display named, its toolkit's X11 code
/// could connect over TCP, which the program never does.
fn start(program: &Path) -> io::Result<Started> {
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
    let (read, write) = UnixStream::from_std(ours)?.into_split();
    Ok(Started {
        process: Box::new(child),
        read: Box::new(read),
        write: Box::new(write),
        // The desktop raises a window as its activation says.
        let_forward: Box::new(|| {}),
    })
}

#[cfg(test)]
mod tests;
