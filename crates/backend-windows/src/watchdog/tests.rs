use std::collections::VecDeque;

use auto_clicker_core::input::fake::{Call, FakeBackend};
use auto_clicker_core::input::{Button, Key};

use super::*;
use crate::delivery::Context;

const OPEN: Context = Context {
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
fn each_owed_release_is_sent_once_and_judged() {
    let mut bytes = [0u8; TABLE_LEN];
    bytes[17] = 1;
    bytes[0x110] = 1;
    // No key or button: never sent.
    bytes[0x2ff] = 1;
    let owed = Owed::parse(&bytes);
    let (mut output, log) = FakeBackend::new();
    // The key's release goes with elevated Task Manager in front.
    let mut probe = Script([ELEVATED, ELEVATED].into());
    let sent = release_through(&owed, &mut output, &mut probe);
    let w = Key::new(17).unwrap();
    assert_eq!(
        log.calls(),
        [
            Call::Key(w, PressState::Up),
            Call::Button(Button::Left, PressState::Up)
        ]
    );
    assert_eq!(
        sent,
        [
            (Held::Key(w), Delivery::Blocked),
            (Held::Button(Button::Left), Delivery::Delivered)
        ]
    );
}

#[test]
fn a_release_windows_refused_is_told_apart() {
    let mut bytes = [0u8; TABLE_LEN];
    bytes[17] = 1;
    let owed = Owed::parse(&bytes);
    let w = Key::new(17).unwrap();
    let (mut output, log) = FakeBackend::new();
    log.fail_once(Call::Key(w, PressState::Up));
    let sent = release_through(&owed, &mut output, &mut Script(VecDeque::new()));
    assert_eq!(sent, [(Held::Key(w), Delivery::Refused)]);
}
