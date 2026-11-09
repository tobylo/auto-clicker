//! Watching `/dev/input` for nodes coming and going, through inotify.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;

use inotify::{EventMask, Inotify, WatchMask};
use tokio::io::unix::AsyncFd;

/// A change to the watched directory.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Created(OsString),
    Attrib(OsString),
    Deleted(OsString),
    /// Events were lost: look at the whole directory again.
    Overflow,
}

/// The inotify watch on one directory.
pub(crate) struct Watch {
    inotify: AsyncFd<Inotify>,
    buffer: Vec<u8>,
}

/// Room for many events per read; each is 16 bytes plus a short name.
const BUFFER_BYTES: usize = 4096;

impl Watch {
    /// Starts watching `dir`. Must run inside a tokio runtime with I/O
    /// enabled.
    pub(crate) fn new(dir: &Path) -> io::Result<Watch> {
        let inotify = Inotify::init()?;
        let mask = WatchMask::CREATE
            | WatchMask::ATTRIB
            | WatchMask::DELETE
            | WatchMask::MOVED_TO
            | WatchMask::MOVED_FROM
            | WatchMask::ONLYDIR;
        inotify.watches().add(dir, mask)?;
        Ok(Watch {
            inotify: register(inotify)?,
            buffer: vec![0; BUFFER_BYTES],
        })
    }

    /// Waits for the next batch of changes.
    ///
    /// # Errors
    /// A read error, or the watch ending because the directory went away.
    pub(crate) async fn changes(&mut self) -> io::Result<Vec<Change>> {
        loop {
            let mut ready = self.inotify.readable_mut().await?;
            // `try_io` clears readiness only on `WouldBlock`, so events left
            // after a full buffer wake the next call at once.
            let buffer = &mut self.buffer;
            let read = ready.try_io(|inotify| {
                let events = inotify.get_mut().read_events(buffer)?;
                events
                    .map(|event| change(event.mask, event.name))
                    .collect::<io::Result<Vec<_>>>()
            });
            if let Ok(result) = read {
                return result.map(|changes| changes.into_iter().flatten().collect());
            }
        }
    }
}

/// Hands the inotify descriptor to tokio's reactor.
#[allow(unsafe_code)]
fn register(inotify: Inotify) -> io::Result<AsyncFd<Inotify>> {
    // SAFETY: `register` needs the descriptor to stay open while the
    // `AsyncFd` holds it. `Inotify` owns its descriptor and closes it only
    // when dropped or consumed by `Inotify::close`; the `AsyncFd` owns the
    // `Inotify` and hands out only references, so neither happens before the
    // `AsyncFd` deregisters it on drop.
    let registered = unsafe { AsyncFd::register(inotify) };
    Ok(registered?)
}

/// The change one event stands for: `None` for one that changes nothing,
/// `Err` when the watch itself ended.
fn change(mask: EventMask, name: Option<&OsStr>) -> io::Result<Option<Change>> {
    if mask.contains(EventMask::Q_OVERFLOW) {
        return Ok(Some(Change::Overflow));
    }
    if mask.intersects(EventMask::IGNORED | EventMask::DELETE_SELF | EventMask::UNMOUNT) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the watched directory went away",
        ));
    }
    let Some(name) = name.map(ToOwned::to_owned) else {
        return Ok(None);
    };
    Ok(
        if mask.intersects(EventMask::CREATE | EventMask::MOVED_TO) {
            Some(Change::Created(name))
        } else if mask.contains(EventMask::ATTRIB) {
            Some(Change::Attrib(name))
        } else if mask.intersects(EventMask::DELETE | EventMask::MOVED_FROM) {
            Some(Change::Deleted(name))
        } else {
            None
        },
    )
}
