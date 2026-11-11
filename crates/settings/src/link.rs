//! The window's end of the socket the daemon started it with, as its
//! standard input. Started any other way, it has nothing to talk to and
//! exits.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;

use auto_clicker_core::window::{MAX_LINE, Request, Update, line, parse};

/// Where the window's requests go.
pub struct Link {
    stream: UnixStream,
}

impl Link {
    /// The socket on standard input, and a second handle to it for
    /// reading; an error when standard input is no socket.
    pub fn from_stdin() -> io::Result<(Link, UnixStream)> {
        Link::from_fd(io::stdin().as_fd().try_clone_to_owned()?)
    }

    /// The socket `fd`, and a second handle to it; an error when it is no
    /// socket.
    fn from_fd(fd: std::os::fd::OwnedFd) -> io::Result<(Link, UnixStream)> {
        let file = std::fs::File::from(fd);
        if !file.metadata()?.file_type().is_socket() {
            return Err(io::Error::other(
                "standard input is no socket: the settings window opens from the tray",
            ));
        }
        let stream = UnixStream::from(std::os::fd::OwnedFd::from(file));
        let reader = stream.try_clone()?;
        Ok((Link { stream }, reader))
    }

    /// Sends `request`. A request is far shorter than a line may be, and
    /// the daemon reads them as they come.
    pub fn send(&self, request: &Request) -> io::Result<()> {
        let bytes = line(request).map_err(io::Error::other)?;
        (&self.stream).write_all(&bytes)
    }
}

/// Reads updates from `read` and hands each to `on`, until the daemon
/// closes its end or sends a line that is no update.
pub fn read_updates(read: impl Read, mut on: impl FnMut(Update)) {
    let mut read = BufReader::new(read);
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        let limit = u64::try_from(MAX_LINE).unwrap_or(u64::MAX);
        match (&mut read).take(limit).read_until(b'\n', &mut bytes) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        // A line cut short, by the limit or by the end, is no line.
        if bytes.pop() != Some(b'\n') {
            return;
        }
        match parse::<Update>(&bytes) {
            Ok(update) => on(update),
            Err(_) => return,
        }
    }
}

/// Reads the requests a window sent on `read`, for tests.
#[cfg(test)]
pub fn read_requests(read: impl Read) -> Vec<Request> {
    BufReader::new(read)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| parse(line.as_bytes()).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use auto_clicker_core::hotkeys::Capture;
    use auto_clicker_core::window::Ask;

    use super::*;

    #[test]
    fn only_a_socket_is_taken() {
        let (ours, _theirs) = UnixStream::pair().unwrap();
        assert!(Link::from_fd(std::os::fd::OwnedFd::from(ours)).is_ok());
        let file = std::fs::File::open("/dev/null").unwrap();
        assert!(Link::from_fd(std::os::fd::OwnedFd::from(file)).is_err());
    }

    #[test]
    fn requests_go_out_one_line_each() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let link = Link { stream: ours };
        for id in [1, 2] {
            let ask = Ask::Capture(Capture::Key);
            link.send(&Request { id, ask }).unwrap();
        }
        drop(link);
        let ids: Vec<u64> = read_requests(theirs).iter().map(|r| r.id).collect();
        assert_eq!(ids, [1, 2]);
    }

    #[test]
    fn updates_are_read_until_the_end_or_a_line_that_is_none() {
        let raise = line(&Update::Raise).unwrap();
        let mut seen = 0;
        read_updates([raise.clone(), raise.clone()].concat().as_slice(), |_| {
            seen += 1
        });
        assert_eq!(seen, 2);
        let mut seen = 0;
        let bytes = [raise.clone(), b"nonsense\n".to_vec(), raise.clone()].concat();
        read_updates(bytes.as_slice(), |_| seen += 1);
        assert_eq!(seen, 1);
        let mut cut = raise.clone();
        cut.pop();
        let mut seen = 0;
        read_updates([raise.clone(), cut].concat().as_slice(), |_| seen += 1);
        assert_eq!(seen, 1);
        let long = [vec![b' '; MAX_LINE], raise.clone()].concat();
        let mut seen = 0;
        read_updates(long.as_slice(), |_| seen += 1);
        assert_eq!(seen, 0);
    }
}
