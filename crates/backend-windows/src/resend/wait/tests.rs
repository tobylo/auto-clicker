use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use auto_clicker_core::input::fake::{Call, FakeBackend};
use auto_clicker_core::input::{Held, Key, PressState};

use super::*;
use crate::delivery::Context;

const OPEN: Context = Context {
    connected: true,
    desktop_has_input: true,
    foreground: 7,
    foreground_allowed: true,
};

/// The desktop as open, telling the test of each question and whether the
/// hooks were on when it was asked.
struct Watching(mpsc::Sender<bool>);

impl Probe for Watching {
    fn context(&mut self) -> Context {
        let _ = self.0.send(hooked_here());
        OPEN
    }
}

fn w() -> Key {
    Key::new(17).unwrap()
}

/// How a wait ended, the calls it sent, and whether the hooks were still
/// on after it.
type Outcome = (Ended, Vec<Call>, bool);

/// Runs the wait for W's release on a thread of its own, its output
/// refusing the release `refusals` times; the stop, the questions the
/// probe was asked, and the wait's end with the calls sent and whether the
/// hooks were still on after it.
fn waiting(refusals: usize) -> (Arc<Stop>, mpsc::Receiver<bool>, thread::JoinHandle<Outcome>) {
    let stop = Arc::new(Stop::new().unwrap());
    let (asked, questions) = mpsc::channel();
    let shared = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        let (output, log) = FakeBackend::new();
        for _ in 0..refusals {
            log.fail_once(Call::Key(w(), PressState::Up));
        }
        let mut resend = Resend::new([Held::Key(w())]);
        let mut via = Via {
            output,
            probe: Watching(asked),
        };
        let ended = send_until_delivered(&mut resend, &mut via, &shared, None);
        (ended, log.calls(), hooked_here())
    });
    (stop, questions, thread)
}

#[test]
fn a_refused_release_is_delivered_by_a_timed_try_with_the_hooks_on_and_then_off() {
    let (_stop, questions, thread) = waiting(1);
    let (ended, calls, hooked_after) = thread.join().unwrap();
    assert!(matches!(ended, Ended::Delivered), "{ended:?}");
    assert_eq!(calls.len(), 2, "the first pass and one timed try");
    let hooked: Vec<bool> = questions.try_iter().collect();
    assert!(
        !hooked.is_empty() && hooked.iter().all(|&on| on),
        "{hooked:?}"
    );
    assert!(!hooked_after, "the hooks outlived what was owed");
}

#[test]
fn a_stop_ends_the_timed_tries_at_once() {
    let (stop, questions, thread) = waiting(100);
    // The first pass was refused; its timed try is 100 ms away.
    questions.recv_timeout(Duration::from_secs(10)).unwrap();
    questions.recv_timeout(Duration::from_secs(10)).unwrap();
    let stopping = Instant::now();
    stop.set();
    let (ended, calls, hooked_after) = thread.join().unwrap();
    let took = stopping.elapsed();
    assert!(matches!(ended, Ended::Stopped), "{ended:?}");
    assert!(took < Duration::from_millis(50), "took {took:?}");
    assert_eq!(calls.len(), 1, "a timed try went after the stop");
    assert!(!hooked_after);
}

/// What the wait also sleeps on: an event, given until it is taken.
struct Flag {
    event: Arc<Stop>,
    taken: Option<mpsc::Sender<bool>>,
}

impl Also for Flag {
    fn handle(&self) -> Option<HANDLE> {
        self.taken.as_ref().map(|_| self.event.raw())
    }

    fn taken(&mut self, abandoned: bool) {
        if let Some(taken) = self.taken.take() {
            let _ = taken.send(abandoned);
        }
    }
}

#[test]
fn what_else_the_wait_sleeps_on_is_taken_once_and_the_wait_goes_on() {
    let stop = Arc::new(Stop::new().unwrap());
    let event = Arc::new(Stop::new().unwrap());
    let (taken, took) = mpsc::channel();
    let (asked, questions) = mpsc::channel();
    let (shared_stop, shared_event) = (Arc::clone(&stop), Arc::clone(&event));
    let thread = thread::spawn(move || {
        let (output, log) = FakeBackend::new();
        // Refused for good: the wait outlives its timed tries.
        for _ in 0..100 {
            log.fail_once(Call::Key(w(), PressState::Up));
        }
        let mut resend = Resend::new([Held::Key(w())]);
        let mut flag = Flag {
            event: shared_event,
            taken: Some(taken),
        };
        let mut via = Via {
            output,
            probe: Watching(asked),
        };
        send_until_delivered(&mut resend, &mut via, &shared_stop, Some(&mut flag))
    });
    questions.recv_timeout(Duration::from_secs(10)).unwrap();
    // Set for good, as a free mutex stays: taken once, then not waited on.
    event.set();
    assert_eq!(took.recv_timeout(Duration::from_secs(10)), Ok(false));
    stop.set();
    let ended = thread.join().unwrap();
    assert!(matches!(ended, Ended::Stopped), "{ended:?}");
}

#[test]
fn a_timeout_ends_after_its_deadline_and_never_waits_for_ever() {
    let now = Instant::now();
    assert_eq!(timeout_until(now), 0);
    let soon = timeout_until(Instant::now() + Duration::from_micros(1500));
    assert!((1..=2).contains(&soon), "{soon}");
    let far = Instant::now() + Duration::from_secs(u64::from(u32::MAX));
    assert_eq!(timeout_until(far), INFINITE - 1);
}
