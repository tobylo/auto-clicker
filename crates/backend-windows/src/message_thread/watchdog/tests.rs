use std::os::windows::io::AsHandle;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use super::*;
use crate::message_thread::tests::{JOIN, next_followed, started};

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

#[test]
fn the_watchdogs_end_reaches_the_handler_at_once() {
    let (thread, received) = started();
    let (mut child, handle) = stand_in();
    let wait = thread.remote().wait_for_watchdog(handle).unwrap();
    child.kill().unwrap();
    let killed = Instant::now();
    assert_eq!(next_followed(&received, 1), ["WatchdogEnded"]);
    assert!(killed.elapsed() < AT_ONCE, "{:?}", killed.elapsed());
    assert!(wait.stop(AT_ONCE));
    let _ = child.wait();
    thread.quit_and_join(JOIN).unwrap();
}

#[test]
fn a_stopped_wait_ends_at_once_and_says_nothing() {
    let (thread, received) = started();
    let (mut child, handle) = stand_in();
    let wait = thread.remote().wait_for_watchdog(handle).unwrap();
    let stopping = Instant::now();
    assert!(wait.stop(AT_ONCE));
    assert!(stopping.elapsed() < AT_ONCE, "{:?}", stopping.elapsed());
    child.kill().unwrap();
    let _ = child.wait();
    thread.quit_and_join(JOIN).unwrap();
    // Only the thread's own end follows.
    assert_eq!(next_followed(&received, 1), ["Ended"]);
}
