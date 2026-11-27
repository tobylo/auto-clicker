use std::sync::mpsc;
use std::time::Instant;

use super::*;

fn unique(what: &str) -> String {
    format!(
        r"Local\io.github.tobylo.AutoClicker.test.{what}.{}.{:?}",
        std::process::id(),
        Instant::now()
    )
}

/// Holds the mutex `name` on a thread of its own until told; with `abandon`
/// the thread ends without letting go.
fn held_elsewhere(name: &str, abandon: bool) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
    let name = name.to_owned();
    let (taken, took) = mpsc::channel();
    let (go, gone) = mpsc::channel();
    let thread = thread::spawn(move || {
        let holding = Holding::take_named(&name).unwrap();
        assert!(holding.held());
        taken.send(()).unwrap();
        let _ = gone.recv();
        if abandon {
            std::mem::forget(holding);
        }
    });
    took.recv_timeout(Duration::from_secs(10)).unwrap();
    (go, thread)
}

/// Waits on the earlier watchdog's mutex `name`, telling the test when it
/// is free.
fn waiting_on(name: &str) -> (EarlierWait, mpsc::Receiver<Option<io::Error>>) {
    let earlier = open_named(name).unwrap().expect("the mutex is there");
    let (free, freed) = mpsc::channel();
    let wait = earlier.wait(move |err| free.send(err).unwrap()).unwrap();
    (wait, freed)
}

#[test]
fn no_earlier_watchdog_is_none() {
    assert!(open_named(&unique("none")).unwrap().is_none());
}

#[test]
#[allow(unsafe_code)]
fn a_daemon_waits_while_an_earlier_watchdog_holds_it_and_lets_go_once_free() {
    let name = unique("held");
    let (go, holder) = held_elsewhere(&name, false);
    // Kept open, so the mutex outlives the others' handles.
    let kept = open_named(&name).unwrap().unwrap();
    let (wait, freed) = waiting_on(&name);
    assert!(freed.recv_timeout(Duration::from_millis(200)).is_err());
    go.send(()).unwrap();
    holder.join().unwrap();
    let free = freed.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(free.is_none());
    assert!(wait.stop(Duration::from_secs(1)));
    // Let go of, not abandoned with the wait's thread.
    // SAFETY: the mutex is open for as long as `kept`; no wait.
    let woken = unsafe { WaitForSingleObject(kept.0.as_raw_handle(), 0) };
    assert_eq!(woken, WAIT_OBJECT_0);
}

#[test]
fn an_abandoned_mutex_counts_as_free() {
    let name = unique("abandoned");
    let (go, holder) = held_elsewhere(&name, true);
    let (wait, freed) = waiting_on(&name);
    go.send(()).unwrap();
    holder.join().unwrap();
    assert!(
        freed
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .is_none()
    );
    drop(wait);
}

#[test]
fn a_stopped_wait_ends_at_once_and_says_nothing() {
    let name = unique("stopped");
    let (go, holder) = held_elsewhere(&name, false);
    let (wait, freed) = waiting_on(&name);
    let stopping = Instant::now();
    assert!(wait.stop(Duration::from_secs(1)));
    let took = stopping.elapsed();
    assert!(took < Duration::from_millis(50), "took {took:?}");
    go.send(()).unwrap();
    holder.join().unwrap();
    assert!(freed.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
#[allow(unsafe_code)]
fn a_second_watchdog_waits_for_the_first_and_then_holds_it() {
    let name = unique("second");
    let (go, holder) = held_elsewhere(&name, false);
    let mut second = Holding::take_named(&name).unwrap();
    assert!(!second.held());
    let handle = second.handle().expect("waited on until taken");
    go.send(()).unwrap();
    holder.join().unwrap();
    // SAFETY: the mutex is open for as long as `second`.
    let woken = unsafe { WaitForSingleObject(handle, 10_000) };
    assert_eq!(woken, WAIT_OBJECT_0);
    second.taken(false);
    assert!(second.held() && second.handle().is_none());
    let (wait, freed) = waiting_on(&name);
    assert!(freed.recv_timeout(Duration::from_millis(200)).is_err());
    // Let go of when dropped.
    drop(second);
    assert!(
        freed
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .is_none()
    );
    drop(wait);
}

#[test]
fn a_dropped_wait_ends_and_says_nothing() {
    let name = unique("dropped");
    let (go, holder) = held_elsewhere(&name, false);
    let (wait, freed) = waiting_on(&name);
    drop(wait);
    go.send(()).unwrap();
    holder.join().unwrap();
    assert!(freed.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn a_watchdog_takes_a_mutex_an_earlier_one_abandoned() {
    let name = unique("taken-abandoned");
    let (go, holder) = held_elsewhere(&name, true);
    // Kept open, so the abandoned mutex outlives its holder's thread.
    let kept = open_named(&name).unwrap().unwrap();
    go.send(()).unwrap();
    holder.join().unwrap();
    let holding = Holding::take_named(&name).unwrap();
    assert!(holding.held() && holding.handle().is_none());
    drop((holding, kept));
}
