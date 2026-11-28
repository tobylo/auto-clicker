//! The window's end of the socket the daemon started it with, as its
//! standard input. Started any other way, it has nothing to talk to and
//! exits.

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::FileTypeExt;

use crate::link::Link;

/// The socket on standard input, and a second handle to it for reading;
/// an error when standard input is no socket.
///
/// The socket moves off standard input, which then reads /dev/null: a
/// program the window starts (a browser for the About tab's link) inherits
/// standard input, and holding the socket open it would keep the daemon
/// from seeing the window close. The handles kept are closed on exec.
pub fn from_stdin() -> io::Result<(Link, File)> {
    let link = from_fd(io::stdin().as_fd().try_clone_to_owned()?)?;
    rustix::stdio::dup2_stdin(File::open("/dev/null")?)?;
    Ok(link)
}

/// The socket `fd`, and a second handle to it; an error when it is no
/// socket.
fn from_fd(fd: OwnedFd) -> io::Result<(Link, File)> {
    let file = File::from(fd);
    if !file.metadata()?.file_type().is_socket() {
        return Err(io::Error::other(
            "standard input is no socket: the settings window opens from the tray",
        ));
    }
    let reader = file.try_clone()?;
    Ok((Link::new(file), reader))
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use auto_clicker_core::hotkeys::Capture;
    use auto_clicker_core::window::{Ask, Request};

    use super::*;
    use crate::link::read_requests;

    /// Set in the test's own copy, which runs with a socket on its
    /// standard input.
    const CHILD: &str = "AUTO_CLICKER_SETTINGS_LINK_CHILD";

    #[test]
    fn the_socket_leaves_standard_input() {
        if std::env::var_os(CHILD).is_some() {
            let (link, _reader) = from_stdin().unwrap();
            let stdin = File::from(io::stdin().as_fd().try_clone_to_owned().unwrap());
            assert!(stdin.metadata().unwrap().file_type().is_char_device());
            let ask = Ask::Capture(Capture::Key);
            link.send(&Request { id: 7, ask }).unwrap();
            return;
        }
        let (ours, theirs) = UnixStream::pair().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "linux::link::tests::the_socket_leaves_standard_input",
                "--exact",
            ])
            .env(CHILD, "1")
            .stdin(OwnedFd::from(theirs))
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        assert!(child.wait().unwrap().success());
        // The child's handles are all closed, so the request ends the read.
        let ids: Vec<u64> = read_requests(ours).iter().map(|r| r.id).collect();
        assert_eq!(ids, [7]);
    }

    #[test]
    fn only_a_socket_is_taken() {
        let (ours, _theirs) = UnixStream::pair().unwrap();
        assert!(from_fd(OwnedFd::from(ours)).is_ok());
        let file = File::open("/dev/null").unwrap();
        assert!(from_fd(OwnedFd::from(file)).is_err());
    }
}
