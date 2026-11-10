//! What each input event from a keyboard means for the matcher.

use std::time::{Duration, UNIX_EPOCH};

use auto_clicker_core::hotkeys::KeyEvent;
use evdev::{EventType, InputEvent, SynchronizationCode};

/// What to do with one event.
pub(crate) enum Step {
    Key(KeyEvent),
    /// Events were lost: what the device holds is unknown until the frame
    /// ends.
    Lost,
    /// Events were lost and the device's frame is over: read which keys it
    /// holds now.
    Resync,
    Nothing,
}

/// The step for `event` from a device whose events were lost (`dropped`)
/// until the end of the current frame. After `SYN_DROPPED` the kernel's
/// queue was cut: everything up to the next `SYN_REPORT` is partial, and the
/// keys held then must be read, not replayed as presses.
pub(crate) fn step(dropped: &mut bool, event: &InputEvent) -> Step {
    if event.event_type() == EventType::SYNCHRONIZATION {
        let code = event.code();
        if code == SynchronizationCode::SYN_DROPPED.0 {
            *dropped = true;
            return Step::Lost;
        } else if code == SynchronizationCode::SYN_REPORT.0 && *dropped {
            *dropped = false;
            return Step::Resync;
        }
        return Step::Nothing;
    }
    match key_event(event) {
        Some(key) if !*dropped => Step::Key(key),
        _ => Step::Nothing,
    }
}

/// Whether the event reports lost events (`SYN_DROPPED`).
pub(crate) fn is_loss(event: &InputEvent) -> bool {
    event.event_type() == EventType::SYNCHRONIZATION
        && event.code() == SynchronizationCode::SYN_DROPPED.0
}

/// The key event an input event is, if it is one.
fn key_event(event: &InputEvent) -> Option<KeyEvent> {
    (event.event_type() == EventType::KEY)
        .then(|| KeyEvent::new(event.code(), event.value(), time(event)))
        .flatten()
}

/// When the event happened, on the monotonic clock every device read is set
/// to. A time that cannot be read is the earliest, so the event cannot fire
/// after a change it might predate.
fn time(event: &InputEvent) -> Duration {
    event
        .timestamp()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
}

/// The monotonic clock now, as event times count it.
#[allow(unsafe_code)]
pub(crate) fn now() -> Duration {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a live, writable `timespec` for the call to fill.
    // `CLOCK_MONOTONIC` always exists, so the call cannot fail; were it to,
    // `now` stays zero, which suppresses nothing.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
    let seconds = u64::try_from(now.tv_sec).unwrap_or(0);
    let nanos = u32::try_from(now.tv_nsec).unwrap_or(0);
    Duration::new(seconds, nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: EventType, code: u16, value: i32) -> InputEvent {
        InputEvent::new(kind.0, code, value)
    }

    fn key(value: i32) -> InputEvent {
        event(EventType::KEY, 64, value)
    }

    fn sync(code: SynchronizationCode) -> InputEvent {
        event(EventType::SYNCHRONIZATION, code.0, 0)
    }

    fn name(step: Step) -> &'static str {
        match step {
            Step::Key(_) => "key",
            Step::Lost => "lost",
            Step::Resync => "resync",
            Step::Nothing => "nothing",
        }
    }

    #[test]
    fn only_key_events_are_matched() {
        let is_key = [
            key(1),
            key(0),
            key(2),
            key(7),
            event(EventType::RELATIVE, 64, 1),
            event(EventType::MISC, 64, 1),
            sync(SynchronizationCode::SYN_REPORT),
        ]
        .map(|event| key_event(&event).is_some());
        assert_eq!(is_key, [true, true, true, false, false, false, false]);
    }

    #[test]
    fn keys_after_lost_events_are_read_once_the_frame_ends() {
        let mut dropped = false;
        let steps = [
            key(1),
            sync(SynchronizationCode::SYN_REPORT),
            sync(SynchronizationCode::SYN_DROPPED),
            key(1),
            event(EventType::MISC, 4, 1),
            sync(SynchronizationCode::SYN_REPORT),
            key(0),
            sync(SynchronizationCode::SYN_REPORT),
        ]
        .map(|event| name(step(&mut dropped, &event)));
        assert_eq!(
            steps,
            [
                "key", "nothing", "lost", "nothing", "nothing", "resync", "key", "nothing"
            ]
        );
    }
}
