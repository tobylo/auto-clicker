//! Trying a file call again while another program holds the file for a
//! moment.
//!
//! On Windows a virus scanner or the search indexer opens a file just
//! written, and while it does, a rename, a link, a removal or an open may
//! fail with a sharing violation. That is transient: the call is tried
//! again, after pauses that double, until one deadline shared by the whole
//! operation, and then reported as busy, never as missing. The deadline
//! starts the first time another program holds a file up, so time spent
//! writing a large file never uses it up. Linux has no such failure, so
//! there every call is made once.

use std::io;
use std::path::Path;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use super::sys;

/// How long one operation (a save, a listing, a deletion) waits in all for
/// files other programs hold. File work runs while the daemon is idle, so
/// no click waits on it; a shutdown waits at most this long.
pub(super) const BUDGET: Duration = Duration::from_millis(300);

/// The first pause before a call is tried again; each later one doubles.
const FIRST_PAUSE: Duration = Duration::from_millis(10);

/// What a call does, which decides what counts as transient: on Windows a
/// change to a file another program holds may fail as "access denied"
/// rather than as a sharing violation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Op {
    Read,
    Change,
}

/// One operation's deadline for calls other programs hold up.
#[derive(Debug)]
pub(super) struct Patience {
    budget: Duration,
    /// Set the first time a call is held up: `budget` from then.
    deadline: OnceLock<Instant>,
}

impl Patience {
    /// A wait of [`BUDGET`] in all.
    pub(super) fn new() -> Patience {
        Patience::lasting(BUDGET)
    }

    pub(super) fn lasting(budget: Duration) -> Patience {
        Patience {
            budget,
            deadline: OnceLock::new(),
        }
    }

    /// The deadline, starting it now if no call was held up before.
    fn deadline(&self) -> Instant {
        *self.deadline.get_or_init(|| {
            let now = Instant::now();
            now.checked_add(self.budget).unwrap_or(now)
        })
    }

    /// `call`'s result, tried again while this system calls its failure
    /// transient and the deadline has not passed.
    pub(super) fn retry<T>(
        &self,
        path: &Path,
        op: Op,
        call: impl FnMut() -> io::Result<T>,
    ) -> io::Result<T> {
        self.retry_when(path, |err| sys::transient(err, op), call)
    }

    /// [`retry`](Self::retry), with `transient` deciding which failures
    /// are tried again.
    pub(super) fn retry_when<T>(
        &self,
        path: &Path,
        transient: impl Fn(&io::Error) -> bool,
        mut call: impl FnMut() -> io::Result<T>,
    ) -> io::Result<T> {
        let mut pause = FIRST_PAUSE;
        loop {
            let err = match call() {
                Err(err) if transient(&err) => err,
                done => return done,
            };
            let left = self.deadline().saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(busy(path, err));
            }
            thread::sleep(pause.min(left));
            pause = pause.saturating_mul(2);
        }
    }
}

/// The error for a call still held up at the deadline. "Access denied" may
/// be a real refusal, so it stays as it is; anything else transient says
/// the file is busy.
fn busy(path: &Path, err: io::Error) -> io::Error {
    if err.kind() == io::ErrorKind::PermissionDenied {
        return err;
    }
    io::Error::new(
        io::ErrorKind::ResourceBusy,
        format!(
            "{} is in use by another program, such as a virus scanner or the search \
             indexer; try again ({err})",
            path.display()
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// What a sharing violation looks like to the caller.
    fn shared() -> io::Error {
        io::Error::other("sharing violation")
    }

    fn is_shared(err: &io::Error) -> bool {
        err.to_string() == "sharing violation"
    }

    #[test]
    fn a_call_held_up_past_the_deadline_is_busy_not_missing() {
        let tries = Cell::new(0_u32);
        let started = Instant::now();
        let err = Patience::lasting(Duration::from_millis(50))
            .retry_when(Path::new("held.json"), is_shared, || -> io::Result<()> {
                tries.set(tries.get() + 1);
                Err(shared())
            })
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ResourceBusy);
        let text = err.to_string();
        assert!(
            text.starts_with("held.json is in use by another program"),
            "{text}"
        );
        assert!(text.ends_with("try again (sharing violation)"), "{text}");
        // Tried again, with pauses: 10, 20, then the 20 ms left.
        assert!((2..=5).contains(&tries.get()), "{}", tries.get());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_call_let_go_within_the_deadline_succeeds() {
        let tries = Cell::new(0_u32);
        let done = Patience::lasting(Duration::from_secs(30)).retry_when(
            Path::new("held.json"),
            is_shared,
            || {
                tries.set(tries.get() + 1);
                if tries.get() < 3 {
                    Err(shared())
                } else {
                    Ok("read")
                }
            },
        );
        assert_eq!((done.unwrap(), tries.get()), ("read", 3));
    }

    #[test]
    fn time_before_the_first_hold_up_does_not_count() {
        // Margins wide enough that a runner pausing this thread for a while
        // between the two tries does not use up the budget.
        let patience = Patience::lasting(Duration::from_secs(2));
        // A large file being written, before anything is held up.
        thread::sleep(Duration::from_millis(2100));
        let tries = Cell::new(0_u32);
        let placed = patience.retry_when(Path::new("held.json"), is_shared, || {
            tries.set(tries.get() + 1);
            if tries.get() < 2 {
                Err(shared())
            } else {
                Ok("placed")
            }
        });
        assert_eq!((placed.unwrap(), tries.get()), ("placed", 2));
    }

    #[test]
    fn other_failures_and_successes_are_returned_at_once() {
        let patience = Patience::lasting(Duration::from_secs(30));
        let tries = Cell::new(0_u32);
        let missing = patience.retry_when(Path::new("x"), is_shared, || -> io::Result<()> {
            tries.set(tries.get() + 1);
            Err(io::ErrorKind::NotFound.into())
        });
        let done = patience.retry_when(Path::new("x"), is_shared, || {
            tries.set(tries.get() + 1);
            Ok("done")
        });
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!((done.unwrap(), tries.get()), ("done", 2));
    }

    #[test]
    fn access_denied_at_the_deadline_stays_denied() {
        let err = Patience::lasting(Duration::ZERO)
            .retry_when(
                Path::new("x"),
                |_| true,
                || -> io::Result<()> { Err(io::ErrorKind::PermissionDenied.into()) },
            )
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn one_deadline_covers_every_call_of_an_operation() {
        let patience = Patience::lasting(Duration::from_millis(30));
        let held = || -> io::Result<()> { Err(shared()) };
        let first = patience.retry_when(Path::new("a"), is_shared, held);
        let tries = Cell::new(0_u32);
        let second = patience.retry_when(Path::new("b"), is_shared, || -> io::Result<()> {
            tries.set(tries.get() + 1);
            Err(shared())
        });
        assert_eq!(first.unwrap_err().kind(), io::ErrorKind::ResourceBusy);
        assert_eq!(second.unwrap_err().kind(), io::ErrorKind::ResourceBusy);
        // The deadline had passed: the second call was made once.
        assert_eq!(tries.get(), 1);
    }

    #[test]
    fn this_system_says_what_is_transient() {
        let patience = Patience::lasting(Duration::from_secs(30));
        let found = patience.retry(Path::new("x"), Op::Read, || -> io::Result<()> {
            Err(io::ErrorKind::NotFound.into())
        });
        assert_eq!(found.unwrap_err().kind(), io::ErrorKind::NotFound);
    }
}
