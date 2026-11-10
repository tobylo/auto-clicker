//! What each input event from a device means for the matcher and the
//! recorder.

use std::time::{Duration, UNIX_EPOCH};

use auto_clicker_core::hotkeys::KeyEvent;
use evdev::{EventType, InputEvent, RelativeAxisCode, SynchronizationCode};

/// What to do with one event.
pub(crate) enum Step {
    /// A key or button event for the matcher, and for a recording its code
    /// and whether it went down; `None` for autorepeat, never recorded.
    Key(KeyEvent, Option<(u16, bool)>),
    /// Relative pointer motion.
    Motion {
        dx: i32,
        dy: i32,
    },
    /// Classic wheel notches. The high-resolution wheel is not read: a
    /// mouse that has it sends these too, and playback turns only these.
    Wheel(i32),
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
    if *dropped {
        return Step::Nothing;
    }
    match (event.event_type(), key_event(event)) {
        (_, Some(key)) => Step::Key(key, key_change(event)),
        (EventType::RELATIVE, None) => relative(event.code(), event.value()),
        _ => Step::Nothing,
    }
}

/// A key's code and whether it went down; `None` for autorepeat.
fn key_change(event: &InputEvent) -> Option<(u16, bool)> {
    match event.value() {
        0 => Some((event.code(), false)),
        1 => Some((event.code(), true)),
        _ => None,
    }
}

fn relative(axis: u16, value: i32) -> Step {
    match RelativeAxisCode(axis) {
        RelativeAxisCode::REL_X => Step::Motion { dx: value, dy: 0 },
        RelativeAxisCode::REL_Y => Step::Motion { dx: 0, dy: value },
        RelativeAxisCode::REL_WHEEL => Step::Wheel(value),
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
pub(crate) fn time(event: &InputEvent) -> Duration {
    event
        .timestamp()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
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
            Step::Key(_, Some((_, true))) => "down",
            Step::Key(_, Some((_, false))) => "up",
            Step::Key(_, None) => "repeat",
            Step::Motion { .. } => "motion",
            Step::Wheel(_) => "wheel",
            Step::Lost => "lost",
            Step::Resync => "resync",
            Step::Nothing => "nothing",
        }
    }

    #[test]
    fn only_a_dropped_report_is_a_loss() {
        let losses = [
            sync(SynchronizationCode::SYN_DROPPED),
            sync(SynchronizationCode::SYN_REPORT),
            event(EventType::KEY, SynchronizationCode::SYN_DROPPED.0, 1),
        ]
        .map(|event| is_loss(&event));
        assert_eq!(losses, [true, false, false]);
    }

    #[test]
    fn an_events_time_is_its_kernel_stamp() {
        let stamped = InputEvent::from(libc::input_event {
            time: libc::timeval {
                tv_sec: 12,
                tv_usec: 345_000,
            },
            type_: EventType::KEY.0,
            code: 64,
            value: 1,
        });
        assert_eq!(time(&stamped), Duration::from_millis(12_345));
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
            rel(RelativeAxisCode::REL_X, 3),
            event(EventType::MISC, 4, 1),
            sync(SynchronizationCode::SYN_REPORT),
            key(0),
            sync(SynchronizationCode::SYN_REPORT),
        ]
        .map(|event| name(step(&mut dropped, &event)));
        assert_eq!(
            steps,
            [
                "down", "nothing", "lost", "nothing", "nothing", "nothing", "resync", "up",
                "nothing"
            ]
        );
    }

    fn rel(axis: RelativeAxisCode, value: i32) -> InputEvent {
        event(EventType::RELATIVE, axis.0, value)
    }

    #[test]
    fn keys_motion_and_the_classic_wheel_are_told_apart() {
        let mut dropped = false;
        let mut read = |event: InputEvent| match step(&mut dropped, &event) {
            Step::Key(_, change) => format!("key {change:?}"),
            Step::Motion { dx, dy } => format!("motion {dx} {dy}"),
            Step::Wheel(notches) => format!("wheel {notches}"),
            Step::Lost | Step::Resync | Step::Nothing => "nothing".to_owned(),
        };
        let steps = [
            key(1),
            key(2),
            key(0),
            rel(RelativeAxisCode::REL_X, -4),
            rel(RelativeAxisCode::REL_Y, 7),
            rel(RelativeAxisCode::REL_WHEEL, -1),
            rel(RelativeAxisCode::REL_WHEEL_HI_RES, -120),
            rel(RelativeAxisCode::REL_HWHEEL, 1),
            event(EventType::ABSOLUTE, 0, 500),
        ]
        .map(&mut read);
        assert_eq!(
            steps,
            [
                "key Some((64, true))",
                "key None",
                "key Some((64, false))",
                "motion -4 0",
                "motion 0 7",
                "wheel -1",
                "nothing",
                "nothing",
                "nothing",
            ]
        );
    }
}
