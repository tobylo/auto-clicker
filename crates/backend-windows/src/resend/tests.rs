use std::collections::VecDeque;

use auto_clicker_core::input::fake::{Call, FakeBackend};
use auto_clicker_core::input::{Button, Key};

use super::*;
use crate::delivery::Context;

const OPEN: Context = Context {
    connected: true,
    desktop_has_input: true,
    foreground: 7,
    foreground_allowed: true,
};
const ELEVATED: Context = Context {
    foreground_allowed: false,
    ..OPEN
};

/// What is in front at each question, in turn; `then` once the list is out.
struct Script {
    next: VecDeque<Context>,
    then: Context,
}

impl Script {
    fn always(context: Context) -> Script {
        Script {
            next: VecDeque::new(),
            then: context,
        }
    }
}

impl Probe for Script {
    fn context(&mut self) -> Context {
        self.next.pop_front().unwrap_or(self.then)
    }
}

fn w() -> Key {
    Key::new(17).unwrap()
}

/// Makes the next `times` attempts of `call` fail.
fn fail(log: &auto_clicker_core::input::fake::FakeLog, call: Call, times: usize) {
    for _ in 0..times {
        log.fail_once(call);
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn nothing_is_sent_while_the_context_cannot_deliver_and_an_event_then_delivers() {
    let mut resend = Resend::new([Held::Key(w()), Held::Button(Button::Left)]);
    let (output, log) = FakeBackend::new();
    let mut via = Via {
        output,
        probe: Script::always(ELEVATED),
    };
    let now = Instant::now();
    let next = resend.pass(Cause::Event, now, &mut via);
    assert_eq!(next, Next::Events);
    assert_eq!(log.calls(), []);
    assert_eq!(resend.owed(), 2);
    // The user's window is back.
    via.probe = Script::always(OPEN);
    let next = resend.pass(Cause::Event, now, &mut via);
    assert_eq!(next, Next::Done);
    assert_eq!(
        log.calls(),
        [
            Call::Key(w(), PressState::Up),
            Call::Button(Button::Left, PressState::Up)
        ]
    );
}

#[test]
fn a_release_the_window_changed_under_stays_owed_and_the_rest_wait() {
    let mut resend = Resend::new([Held::Key(w()), Held::Button(Button::Left)]);
    let (output, log) = FakeBackend::new();
    // Elevated Task Manager came in front while the key's release went.
    let probe = Script {
        next: [OPEN, ELEVATED].into(),
        then: ELEVATED,
    };
    let mut via = Via { output, probe };
    let next = resend.pass(Cause::Event, Instant::now(), &mut via);
    assert_eq!(next, Next::Events);
    // The button was not sent into a context that cannot take it.
    assert_eq!(log.calls(), [Call::Key(w(), PressState::Up)]);
    assert_eq!(resend.owed(), 2);
}

#[test]
fn a_refused_release_gets_five_timed_tries_doubling_then_events_only() {
    let mut resend = Resend::new([Held::Key(w())]);
    let (output, log) = FakeBackend::new();
    let probe = Script::always(OPEN);
    let mut via = Via { output, probe };
    let start = Instant::now();
    fail(&log, Call::Key(w(), PressState::Up), 7);
    let mut now = start;
    let mut waits = Vec::new();
    let mut next = resend.pass(Cause::Event, now, &mut via);
    while let Next::Until(due) = next {
        waits.push(due - now);
        now = due;
        next = resend.pass(Cause::Timer, now, &mut via);
    }
    assert_eq!(waits, [ms(100), ms(200), ms(400), ms(800), ms(1600)]);
    assert_eq!(next, Next::Events);
    // The first try and five timed ones; an event then tries again.
    assert_eq!(log.calls().len(), 6);
    let next = resend.pass(Cause::Event, now, &mut via);
    assert_eq!(next, Next::Events, "an event gives no timed try back");
    let next = resend.pass(Cause::Event, now, &mut via);
    assert_eq!(next, Next::Done);
    assert_eq!(log.calls().len(), 8);
}

#[test]
fn an_event_between_timed_tries_tries_again_and_keeps_the_timer() {
    let mut resend = Resend::new([Held::Key(w())]);
    let (output, log) = FakeBackend::new();
    let probe = Script::always(OPEN);
    let mut via = Via { output, probe };
    let start = Instant::now();
    fail(&log, Call::Key(w(), PressState::Up), 2);
    let next = resend.pass(Cause::Event, start, &mut via);
    assert_eq!(next, Next::Until(start + ms(100)));
    // An event 10 ms in, refused again: the timed try stays where it was.
    let next = resend.pass(Cause::Event, start + ms(10), &mut via);
    assert_eq!(next, Next::Until(start + ms(100)));
    let next = resend.pass(Cause::Timer, start + ms(100), &mut via);
    assert_eq!(next, Next::Done);
    assert_eq!(log.calls().len(), 3);
}

#[test]
fn a_timer_tries_only_the_releases_due() {
    let x = Key::new(45).unwrap();
    let mut resend = Resend::new([Held::Key(w()), Held::Key(x)]);
    let (output, log) = FakeBackend::new();
    let start = Instant::now();
    // W is refused; X goes with an elevated window in front after it.
    log.fail_once(Call::Key(w(), PressState::Up));
    let probe = Script {
        next: [OPEN, OPEN, ELEVATED].into(),
        then: OPEN,
    };
    let mut via = Via { output, probe };
    let next = resend.pass(Cause::Event, start, &mut via);
    assert_eq!(next, Next::Until(start + ms(100)));
    assert_eq!(resend.owed(), 2);
    let next = resend.pass(Cause::Timer, start + ms(100), &mut via);
    // W went; X, blocked, waits for an event.
    assert_eq!(next, Next::Events);
    assert_eq!(
        log.calls(),
        [
            Call::Key(w(), PressState::Up),
            Call::Key(x, PressState::Up),
            Call::Key(w(), PressState::Up)
        ]
    );
}

#[test]
fn a_timed_try_the_context_blocks_waits_for_an_event_and_counts() {
    let mut resend = Resend::new([Held::Key(w())]);
    let (output, log) = FakeBackend::new();
    let start = Instant::now();
    fail(&log, Call::Key(w(), PressState::Up), 10);
    let mut via = Via {
        output,
        probe: Script::always(OPEN),
    };
    let next = resend.pass(Cause::Event, start, &mut via);
    assert_eq!(next, Next::Until(start + ms(100)));
    via.probe = Script::always(ELEVATED);
    let next = resend.pass(Cause::Timer, start + ms(100), &mut via);
    assert_eq!(next, Next::Events);
    assert_eq!(log.calls().len(), 1);
    // Back, refused again: the second timed try, 200 ms on.
    let at = start + ms(150);
    via.probe = Script::always(OPEN);
    let next = resend.pass(Cause::Event, at, &mut via);
    assert_eq!(next, Next::Until(at + ms(200)));
}

#[test]
fn the_timed_tries_end_however_late_they_are_counted() {
    assert_eq!(next_try(TIMED_TRIES, Instant::now()), None);
    assert_eq!(next_try(u32::MAX, Instant::now()), None);
    let now = Instant::now();
    assert_eq!(next_try(4, now), Some(now + ms(1600)));
}
