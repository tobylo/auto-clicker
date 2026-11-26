use std::os::windows::io::AsHandle;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use super::*;

/// Far below the waits these tests interrupt (forever), far above what
/// stopping takes.
const AT_ONCE: Duration = Duration::from_secs(2);

/// A process that runs until it is killed: a stand-in for the watchdog.
fn stand_in() -> (Child, OwnedHandle) {
    let child = Command::new("cmd")
        .args(["/c", "pause"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let handle = child.as_handle().try_clone_to_owned().unwrap();
    (child, handle)
}

/// A wait on `handle` that sends on the channel it returns when it calls.
fn waiting(handle: OwnedHandle) -> (WatchdogWait, mpsc::Receiver<()>) {
    let (called, calls) = mpsc::channel();
    let wait = wait_for_watchdog(handle, move || called.send(()).unwrap()).unwrap();
    (wait, calls)
}

#[test]
fn the_watchdogs_end_is_told_at_once() {
    let (mut child, handle) = stand_in();
    let (wait, calls) = waiting(handle);
    child.kill().unwrap();
    let killed = Instant::now();
    calls.recv_timeout(AT_ONCE).unwrap();
    assert!(killed.elapsed() < AT_ONCE, "{:?}", killed.elapsed());
    assert!(wait.stop(AT_ONCE));
    // Told once.
    assert!(calls.try_recv().is_err());
    let _ = child.wait();
}

#[test]
fn a_stopped_wait_ends_at_once_and_says_nothing() {
    let (mut child, handle) = stand_in();
    let (wait, calls) = waiting(handle);
    let stopping = Instant::now();
    assert!(wait.stop(AT_ONCE));
    assert!(stopping.elapsed() < AT_ONCE, "{:?}", stopping.elapsed());
    child.kill().unwrap();
    let _ = child.wait();
    // The thread is gone, and with it the sender: nothing was told.
    assert_eq!(calls.recv(), Err(mpsc::RecvError));
}

#[test]
fn a_wait_that_fails_counts_as_the_watchdogs_end() {
    // A section cannot be waited on: the wait fails at once.
    let section = crate::watchdog::Section::create().unwrap();
    // SAFETY: the section's handle stays open while it is borrowed.
    #[allow(unsafe_code)]
    let borrowed = unsafe { std::os::windows::io::BorrowedHandle::borrow_raw(section.raw()) };
    let handle = borrowed.try_clone_to_owned().unwrap();
    let (wait, calls) = waiting(handle);
    calls.recv_timeout(AT_ONCE).unwrap();
    assert!(wait.stop(AT_ONCE));
}
