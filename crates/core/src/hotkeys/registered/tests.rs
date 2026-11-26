use std::io;

use super::*;
use crate::hotkeys::Bindings;
use HotkeyEvent::{Fired, ModifiersReleased, ReleaseLost};

const CTRL: u16 = 29;
const SHIFT: u16 = 42;
const RIGHT_CTRL: u16 = 97;
const F6: u16 = 64;
const F7: u16 = 65;
const W: u16 = 17;

const CLICKING: Action = Action::ToggleClicking;
const RECORDING: Action = Action::ToggleRecording;

/// The keyboard as the backend reads it: whether raw input is registered,
/// and what the system's key state says.
#[derive(Default)]
struct FakeKeyboard {
    reading: bool,
    starts: u32,
    refuse: bool,
    down: Vec<u16>,
}

impl Keyboard for FakeKeyboard {
    fn start(&mut self) -> io::Result<()> {
        assert!(!self.reading, "registered twice");
        if self.refuse {
            return Err(io::Error::other("refused"));
        }
        self.reading = true;
        self.starts += 1;
        Ok(())
    }

    fn stop(&mut self) {
        assert!(self.reading, "removed while not registered");
        self.reading = false;
    }

    fn held(&self, code: u16) -> bool {
        self.down.contains(&code)
    }
}

fn registered() -> Registered {
    Registered::new(Bindings::default().actions().to_vec())
}

fn holding(codes: &[u16]) -> FakeKeyboard {
    FakeKeyboard {
        down: codes.to_vec(),
        ..FakeKeyboard::default()
    }
}

fn stops(actions: &[Action]) -> Watching {
    Watching {
        release: false,
        stops: actions.to_vec(),
    }
}

const WAITING: Watching = Watching {
    release: true,
    stops: Vec::new(),
};

/// Presses Ctrl+Shift+`key` in raw input and lets go of it again; what
/// each step sent.
fn press(hotkeys: &mut Registered, keyboard: &mut FakeKeyboard, key: u16) -> Vec<HotkeyEvent> {
    let mut events = Vec::new();
    for (code, down) in [
        (CTRL, true),
        (SHIFT, true),
        (key, true),
        (key, false),
        (SHIFT, false),
        (CTRL, false),
    ] {
        events.extend(hotkeys.key(code, down, keyboard));
    }
    events
}

#[test]
fn nothing_is_read_at_idle() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    assert!(!hotkeys.reading());
    assert_eq!(hotkeys.watch(Watching::default(), &mut keyboard), []);
    assert!(!keyboard.reading && keyboard.starts == 0);
    // A key event that reaches it anyway changes nothing and fires nothing.
    assert_eq!(press(&mut hotkeys, &mut keyboard, F6), []);
}

#[test]
fn a_hotkey_reads_its_modifiers_until_they_are_up_then_stops_reading() {
    let mut keyboard = holding(&[CTRL, SHIFT]);
    let mut hotkeys = registered();
    assert_eq!(hotkeys.hotkey(CLICKING, &mut keyboard), [Fired(CLICKING)]);
    assert!(keyboard.reading, "the wait reads the keyboard");
    assert_eq!(hotkeys.watch(WAITING, &mut keyboard), []);
    assert_eq!(hotkeys.key(F6, false, &mut keyboard), []);
    assert_eq!(hotkeys.key(SHIFT, false, &mut keyboard), []);
    assert!(keyboard.reading);
    assert_eq!(hotkeys.key(CTRL, false, &mut keyboard), [ModifiersReleased]);
    assert!(!keyboard.reading, "nothing needs the keyboard any more");
    // The controller's answer to the release comes after it: no second
    // wait.
    assert_eq!(hotkeys.watch(Watching::default(), &mut keyboard), []);
    assert_eq!(keyboard.starts, 1);
}

#[test]
fn modifiers_already_up_when_the_wait_reads_them_are_released_at_once() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    assert_eq!(
        hotkeys.hotkey(CLICKING, &mut keyboard),
        [Fired(CLICKING), ModifiersReleased]
    );
    assert!(!keyboard.reading);
    // The controller waited on the hotkey before it heard of the release.
    assert_eq!(hotkeys.watch(WAITING, &mut keyboard), []);
    assert_eq!(keyboard.starts, 1, "no second read of a reported release");
}

#[test]
fn a_wait_the_keyboard_cannot_be_read_for_is_lost() {
    let mut keyboard = FakeKeyboard {
        refuse: true,
        ..holding(&[CTRL])
    };
    let mut hotkeys = registered();
    assert_eq!(
        hotkeys.hotkey(CLICKING, &mut keyboard),
        [Fired(CLICKING), ReleaseLost]
    );
    assert!(!hotkeys.reading());
}

#[test]
fn the_controller_ending_its_wait_stops_the_reading_at_once() {
    // The 30 s cancel, a stop, a lock and sleep all leave the wait the
    // same way: the controller no longer waits.
    let mut keyboard = holding(&[CTRL, SHIFT]);
    let mut hotkeys = registered();
    hotkeys.hotkey(CLICKING, &mut keyboard);
    hotkeys.watch(WAITING, &mut keyboard);
    assert_eq!(hotkeys.watch(Watching::default(), &mut keyboard), []);
    assert!(!keyboard.reading);
    // The keys coming up later say nothing.
    assert_eq!(hotkeys.key(CTRL, false, &mut keyboard), []);
}

#[test]
fn the_readers_end_stops_the_reading() {
    let mut keyboard = holding(&[CTRL]);
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    hotkeys.hotkey(RECORDING, &mut keyboard);
    hotkeys.end(&mut keyboard);
    assert!(!keyboard.reading && !hotkeys.reading());
}

#[test]
fn a_wait_ended_by_an_older_answer_is_read_again_when_the_controller_waits() {
    let mut keyboard = holding(&[CTRL, SHIFT]);
    let mut hotkeys = registered();
    // The first start is cancelled: the controller's "no wait" is on its
    // way when the second press arrives.
    hotkeys.hotkey(CLICKING, &mut keyboard);
    hotkeys.watch(WAITING, &mut keyboard);
    hotkeys.hotkey(CLICKING, &mut keyboard);
    hotkeys.hotkey(CLICKING, &mut keyboard);
    assert_eq!(hotkeys.watch(Watching::default(), &mut keyboard), []);
    assert!(!keyboard.reading);
    // Then it waits for the third press: read afresh.
    assert_eq!(hotkeys.watch(WAITING, &mut keyboard), []);
    assert!(keyboard.reading);
    assert_eq!(keyboard.starts, 2);
    hotkeys.key(SHIFT, false, &mut keyboard);
    assert_eq!(hotkeys.key(CTRL, false, &mut keyboard), [ModifiersReleased]);
    assert!(!keyboard.reading);
}

#[test]
fn a_wait_ending_during_a_run_leaves_the_runs_stop_matching_reading() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    assert!(keyboard.reading);
    // The recording hotkey during clicking: its wait joins and ends.
    hotkeys.key(CTRL, true, &mut keyboard);
    hotkeys.key(SHIFT, true, &mut keyboard);
    assert_eq!(hotkeys.hotkey(RECORDING, &mut keyboard), [Fired(RECORDING)]);
    hotkeys.key(SHIFT, false, &mut keyboard);
    assert_eq!(hotkeys.key(CTRL, false, &mut keyboard), [ModifiersReleased]);
    assert!(keyboard.reading, "the run's stop matching still reads");
    assert_eq!(keyboard.starts, 1, "one registration for both");
    // And it still stops the run.
    assert_eq!(
        press(&mut hotkeys, &mut keyboard, F6),
        [Fired(CLICKING), ModifiersReleased]
    );
    hotkeys.watch(Watching::default(), &mut keyboard);
    assert!(!keyboard.reading);
}

#[test]
fn only_the_stop_combinations_of_the_run_fire_from_raw_input() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    // Another hotkey's combination, the key with fewer or more modifiers,
    // and another key.
    assert_eq!(press(&mut hotkeys, &mut keyboard, F7), []);
    for keys in [&[CTRL, F6][..], &[F6], &[CTRL, SHIFT, RIGHT_CTRL, W]] {
        for &code in keys {
            assert_eq!(hotkeys.key(code, true, &mut keyboard), [], "{keys:?}");
        }
        for &code in keys.iter().rev() {
            assert_eq!(hotkeys.key(code, false, &mut keyboard), [], "{keys:?}");
        }
    }
    // Right Ctrl is Ctrl.
    for code in [RIGHT_CTRL, SHIFT] {
        hotkeys.key(code, true, &mut keyboard);
    }
    assert_eq!(hotkeys.key(F6, true, &mut keyboard), [Fired(CLICKING)]);
    // Any of the three during playback.
    hotkeys.watch(
        stops(&[CLICKING, RECORDING, Action::TogglePlayback]),
        &mut keyboard,
    );
    hotkeys.key(F6, false, &mut keyboard);
    assert_eq!(hotkeys.key(F7, true, &mut keyboard), [Fired(RECORDING)]);
}

#[test]
fn a_held_key_repeating_fires_once() {
    let mut keyboard = holding(&[CTRL, SHIFT, F6]);
    let mut hotkeys = registered();
    // Held since before the run began, as after playback started with the
    // modifiers still down: its repeats are no press.
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    assert_eq!(hotkeys.key(F6, true, &mut keyboard), []);
    hotkeys.key(F6, false, &mut keyboard);
    assert_eq!(hotkeys.key(F6, true, &mut keyboard), [Fired(CLICKING)]);
    assert_eq!(hotkeys.key(F6, true, &mut keyboard), []);
}

#[test]
fn one_press_seen_both_ways_counts_once_in_either_order() {
    // Raw input first, then the hotkey.
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    hotkeys.key(CTRL, true, &mut keyboard);
    hotkeys.key(SHIFT, true, &mut keyboard);
    assert_eq!(hotkeys.key(F6, true, &mut keyboard), [Fired(CLICKING)]);
    assert_eq!(hotkeys.hotkey(CLICKING, &mut keyboard), []);
    // The hotkey first, while its raw events are still queued: the key
    // state says the keys are up (an instant injected press).
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    assert_eq!(
        hotkeys.hotkey(CLICKING, &mut keyboard),
        [Fired(CLICKING), ModifiersReleased]
    );
    hotkeys.key(CTRL, true, &mut keyboard);
    hotkeys.key(SHIFT, true, &mut keyboard);
    assert_eq!(hotkeys.key(F6, true, &mut keyboard), []);
}

#[test]
fn a_later_press_is_never_taken_for_an_earlier_ones_other_half() {
    // A game switched the hotkeys off: raw input alone saw the stop. The
    // next press, the game gone, comes as a hotkey.
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    assert_eq!(
        press(&mut hotkeys, &mut keyboard, F6),
        [Fired(CLICKING), ModifiersReleased]
    );
    assert_eq!(
        hotkeys.hotkey(CLICKING, &mut keyboard),
        [Fired(CLICKING), ModifiersReleased]
    );
    // The controller's answer to the stop ends the stop matching before
    // the press's own hotkey arrives: the keyboard is still read until the
    // key comes up, so the hotkey is still the same press.
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    for code in [CTRL, SHIFT, F6] {
        hotkeys.key(code, true, &mut keyboard);
    }
    hotkeys.watch(Watching::default(), &mut keyboard);
    assert!(keyboard.reading);
    assert_eq!(hotkeys.hotkey(CLICKING, &mut keyboard), []);
    for code in [F6, SHIFT, CTRL] {
        hotkeys.key(code, false, &mut keyboard);
    }
    assert!(!keyboard.reading);
    assert_eq!(
        hotkeys.hotkey(CLICKING, &mut keyboard),
        [Fired(CLICKING), ModifiersReleased]
    );
    // A hotkey whose raw half never came (the system kept the key-down
    // from raw input): the key coming up ends its wait for one.
    let mut keyboard = holding(&[CTRL, SHIFT, F6]);
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    keyboard.down.clear();
    hotkeys.key(F6, false, &mut keyboard);
    assert_eq!(hotkeys.hotkey(CLICKING, &mut keyboard), [Fired(CLICKING)]);
    hotkeys.key(F6, false, &mut keyboard);
    assert_eq!(hotkeys.key(F6, true, &mut keyboard), [Fired(CLICKING)]);
}

#[test]
fn a_hotkey_windows_refused_is_neither_delivered_nor_matched() {
    let bindings = Bindings::default();
    let mut hotkeys = Registered::new(vec![(RECORDING, bindings.toggle_recording)]);
    let mut keyboard = holding(&[CTRL, SHIFT]);
    assert_eq!(hotkeys.hotkey(CLICKING, &mut keyboard), []);
    assert!(!keyboard.reading);
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    assert_eq!(hotkeys.key(F6, true, &mut keyboard), []);
}
