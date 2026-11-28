//! The window's link to the daemon: requests go out on one handle, updates
//! come in on another, one JSON line each (see
//! [`auto_clicker_core::window`]). Each system hands the window its handles
//! its own way: a socket on standard input on Linux, two pipes named on the
//! command line on Windows.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};

use auto_clicker_core::window::{MAX_LINE, Request, Update, line, parse};

/// Where the window's requests go.
pub struct Link {
    out: File,
}

impl Link {
    /// Requests go to `out`.
    pub fn new(out: File) -> Link {
        Link { out }
    }

    /// A link over a new pipe, and the pipe's other end.
    #[cfg(test)]
    pub fn pair() -> io::Result<(Link, io::PipeReader)> {
        let (read, write) = io::pipe()?;
        #[cfg(unix)]
        let out = File::from(std::os::fd::OwnedFd::from(write));
        #[cfg(windows)]
        let out = File::from(std::os::windows::io::OwnedHandle::from(write));
        Ok((Link::new(out), read))
    }

    /// Sends `request`. A request is far shorter than a line may be, and
    /// the daemon reads them as they come.
    pub fn send(&self, request: &Request) -> io::Result<()> {
        let bytes = line(request).map_err(io::Error::other)?;
        (&self.out).write_all(&bytes)
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
    fn requests_go_out_one_line_each() {
        let (link, theirs) = Link::pair().unwrap();
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
