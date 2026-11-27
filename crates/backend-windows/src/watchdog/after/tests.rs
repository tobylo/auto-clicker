use std::collections::VecDeque;

use auto_clicker_core::input::fake::{Call, FakeBackend};
use auto_clicker_core::input::{Button, Key, PressState};

use super::*;
use crate::delivery::Context;
use crate::watchdog::TABLE_LEN;

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

/// What is in front at each question, in turn; open once the list is out.
struct Script(VecDeque<Context>);

impl Probe for Script {
    fn context(&mut self) -> Context {
        self.0.pop_front().unwrap_or(OPEN)
    }
}

#[test]
fn each_owed_release_is_sent_at_once_and_what_may_not_have_arrived_stays_owed() {
    let mut bytes = [0u8; TABLE_LEN];
    bytes[17] = 1;
    bytes[0x110] = 1;
    // No key or button: never sent.
    bytes[0x2ff] = 1;
    let owed = Owed::parse(&bytes);
    let (output, log) = FakeBackend::new();
    // Elevated Task Manager comes in front as the key's release goes.
    let probe = Script([OPEN, ELEVATED].into());
    let released = release_through(owed.items(), &mut Via { output, probe });
    let w = Key::new(17).unwrap();
    // The button is not sent into a window that cannot take it.
    assert_eq!(log.calls(), [Call::Key(w, PressState::Up)]);
    assert_eq!((released.owed, released.missed()), (2, 2));
}

#[test]
fn releases_that_arrived_leave_nothing_to_stay_for() {
    let owed = [Held::Key(Key::new(17).unwrap()), Held::Button(Button::Left)];
    let (output, log) = FakeBackend::new();
    let probe = Script(VecDeque::new());
    let released = release_through(&owed, &mut Via { output, probe });
    assert_eq!(log.calls().len(), 2);
    assert_eq!(released.missed(), 0);
    // Nothing to stay for: no window, no hooks, no mutex.
    assert!(matches!(released.send_until_delivered(), Stayed::Delivered));
}

#[test]
fn a_release_windows_refused_stays_owed() {
    let w = Key::new(17).unwrap();
    let (output, log) = FakeBackend::new();
    log.fail_once(Call::Key(w, PressState::Up));
    let probe = Script(VecDeque::new());
    let released = release_through(&[Held::Key(w)], &mut Via { output, probe });
    assert_eq!(released.missed(), 1);
}

#[test]
fn an_unlock_or_a_reconnect_wakes_a_pass_and_the_sessions_end_stops_the_wait() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WTS_CONSOLE_CONNECT, WTS_REMOTE_CONNECT, WTS_SESSION_LOCK, WTS_SESSION_LOGON,
        WTS_SESSION_UNLOCK,
    };
    let notice = |code: u32| answer(WM_WTSSESSION_CHANGE, code as WPARAM);
    let _ = crate::resend::take_signals_here();
    for code in [WTS_SESSION_UNLOCK, WTS_CONSOLE_CONNECT, WTS_REMOTE_CONNECT] {
        assert_eq!(notice(code), Some(0));
        assert_eq!(crate::resend::take_signals_here(), (true, false), "{code}");
    }
    for code in [WTS_SESSION_LOCK, WTS_SESSION_LOGON] {
        assert_eq!(notice(code), Some(0));
        assert_eq!(crate::resend::take_signals_here(), (false, false), "{code}");
    }
    assert_eq!(answer(WM_QUERYENDSESSION, 0), Some(1));
    // A session end called off ends nothing.
    assert_eq!(answer(WM_ENDSESSION, 0), Some(0));
    assert_eq!(crate::resend::take_signals_here(), (false, false));
    assert_eq!(answer(WM_ENDSESSION, 1), Some(0));
    assert_eq!(crate::resend::take_signals_here(), (false, true));
}
