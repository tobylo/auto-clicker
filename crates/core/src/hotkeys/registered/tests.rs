use std::io;

use super::*;
use crate::hotkeys::Bindings;
use crate::input::Key;
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

/// Each hotkey set off by its own key alone.
fn own_key(binding: Binding) -> Vec<Key> {
    vec![binding.key()]
}

fn registered() -> Registered {
    Registered::new(Bindings::default().actions().to_vec(), own_key)
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

/// What one step sent, and whether the keyboard is read after it.
fn after(events: Vec<HotkeyEvent>, keyboard: &FakeKeyboard) -> (Vec<HotkeyEvent>, bool) {
    (events, keyboard.reading)
}

#[test]
fn nothing_is_read_at_idle() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    assert!(!hotkeys.reading());
    let watched = hotkeys.watch(Watching::default(), &mut keyboard);
    assert_eq!(after(watched, &keyboard), (vec![], false));
    // A key event that reaches it anyway changes nothing and fires nothing.
    let pressed = press(&mut hotkeys, &mut keyboard, F6);
    assert_eq!((pressed, keyboard.starts), (vec![], 0));
}

#[test]
fn a_hotkey_reads_its_modifiers_until_they_are_up_then_stops_reading() {
    let mut keyboard = holding(&[CTRL, SHIFT]);
    let mut hotkeys = registered();
    let fired = hotkeys.hotkey(CLICKING, &mut keyboard);
    assert_eq!(after(fired, &keyboard), (vec![Fired(CLICKING)], true));
    hotkeys.watch(WAITING, &mut keyboard);
    hotkeys.key(F6, false, &mut keyboard);
    let shift = hotkeys.key(SHIFT, false, &mut keyboard);
    assert_eq!(after(shift, &keyboard), (vec![], true));
    let ctrl = hotkeys.key(CTRL, false, &mut keyboard);
    assert_eq!(after(ctrl, &keyboard), (vec![ModifiersReleased], false));
    // The controller's answer to the release comes after it: no second
    // wait.
    let answered = hotkeys.watch(Watching::default(), &mut keyboard);
    assert_eq!((answered, keyboard.starts), (vec![], 1));
}

#[test]
fn modifiers_already_up_when_the_wait_reads_them_are_released_at_once() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    let fired = hotkeys.hotkey(CLICKING, &mut keyboard);
    let expected = vec![Fired(CLICKING), ModifiersReleased];
    assert_eq!(after(fired, &keyboard), (expected, false));
    // The controller waited on the hotkey before it heard of the release:
    // no second read of a reported release.
    let waited = hotkeys.watch(WAITING, &mut keyboard);
    assert_eq!((waited, keyboard.starts), (vec![], 1));
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
    let ended = hotkeys.watch(Watching::default(), &mut keyboard);
    assert_eq!(after(ended, &keyboard), (vec![], false));
    // Then it waits for the third press: read afresh.
    let waited = hotkeys.watch(WAITING, &mut keyboard);
    assert_eq!(
        (waited, keyboard.reading, keyboard.starts),
        (vec![], true, 2)
    );
    hotkeys.key(SHIFT, false, &mut keyboard);
    let ctrl = hotkeys.key(CTRL, false, &mut keyboard);
    assert_eq!(after(ctrl, &keyboard), (vec![ModifiersReleased], false));
}

#[test]
fn a_wait_ending_during_a_run_leaves_the_runs_stop_matching_reading() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    // The recording hotkey during clicking: its wait joins and ends.
    hotkeys.key(CTRL, true, &mut keyboard);
    hotkeys.key(SHIFT, true, &mut keyboard);
    let fired = hotkeys.hotkey(RECORDING, &mut keyboard);
    assert_eq!(fired, [Fired(RECORDING)]);
    hotkeys.key(SHIFT, false, &mut keyboard);
    let ctrl = hotkeys.key(CTRL, false, &mut keyboard);
    // The run's stop matching still reads, on the one registration.
    let state = (ctrl, keyboard.reading, keyboard.starts);
    assert_eq!(state, (vec![ModifiersReleased], true, 1));
    // And it still stops the run.
    let stopped = press(&mut hotkeys, &mut keyboard, F6);
    assert_eq!(stopped, [Fired(CLICKING), ModifiersReleased]);
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
    let mut hotkeys = Registered::new(vec![(RECORDING, bindings.toggle_recording)], own_key);
    let mut keyboard = holding(&[CTRL, SHIFT]);
    assert_eq!(hotkeys.hotkey(CLICKING, &mut keyboard), []);
    assert!(!keyboard.reading);
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    assert_eq!(hotkeys.key(F6, true, &mut keyboard), []);
}

#[test]
fn a_hotkey_while_reading_counts_modifiers_whose_raw_events_are_still_queued() {
    // A run's stops are read; the hotkey arrives before the raw Ctrl and
    // Shift downs queued behind it, which the key state already shows.
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    keyboard.down = vec![CTRL, SHIFT];
    let fired = hotkeys.hotkey(RECORDING, &mut keyboard);
    assert_eq!(
        fired,
        [Fired(RECORDING)],
        "no release with Ctrl and Shift down"
    );
    hotkeys.key(CTRL, true, &mut keyboard);
    hotkeys.key(SHIFT, true, &mut keyboard);
    hotkeys.key(SHIFT, false, &mut keyboard);
    assert_eq!(hotkeys.key(CTRL, false, &mut keyboard), [ModifiersReleased]);
}

#[test]
fn the_readers_end_with_a_raw_stop_still_held_stops_the_reading() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    for code in [CTRL, SHIFT, F6] {
        hotkeys.key(code, true, &mut keyboard);
    }
    hotkeys.end(&mut keyboard);
    assert!(!keyboard.reading && !hotkeys.reading());
}

#[test]
fn a_modifier_let_go_first_leaves_the_press_waiting_for_its_hotkey() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    for code in [CTRL, SHIFT, F6] {
        hotkeys.key(code, true, &mut keyboard);
    }
    // Shift comes up before the press's hotkey is handled; F6 is down.
    hotkeys.key(SHIFT, false, &mut keyboard);
    assert_eq!(hotkeys.hotkey(CLICKING, &mut keyboard), []);
}

#[test]
fn a_stop_matches_when_only_its_hotkey_was_registered() {
    let bindings = Bindings::default();
    let mut hotkeys = Registered::new(vec![(CLICKING, bindings.toggle_clicking)], own_key);
    let mut keyboard = FakeKeyboard::default();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    assert!(hotkeys.reading());
    assert_eq!(
        press(&mut hotkeys, &mut keyboard, F6),
        [Fired(CLICKING), ModifiersReleased]
    );
}

#[test]
fn stray_keys_while_raw_input_was_refused_fire_nothing() {
    let mut keyboard = FakeKeyboard {
        refuse: true,
        ..FakeKeyboard::default()
    };
    let mut hotkeys = registered();
    // No release is owed, so a refusal loses none.
    assert_eq!(hotkeys.watch(stops(&[CLICKING]), &mut keyboard), []);
    assert!(!hotkeys.reading());
    assert_eq!(press(&mut hotkeys, &mut keyboard, F6), []);
}

#[test]
fn a_hotkey_heard_before_reading_swallows_no_later_press() {
    // Playback's 2 s wait ran out with Ctrl and Shift still down, F8 up.
    let mut keyboard = holding(&[CTRL, SHIFT]);
    let mut hotkeys = registered();
    let playback = Action::TogglePlayback;
    assert_eq!(hotkeys.hotkey(playback, &mut keyboard), [Fired(playback)]);
    hotkeys.watch(WAITING, &mut keyboard);
    hotkeys.watch(stops(&Action::ALL), &mut keyboard);
    // The next F8 is the user's stop.
    assert_eq!(hotkeys.key(66, true, &mut keyboard), [Fired(playback)]);
}

#[test]
fn a_hotkeys_unpaired_half_is_forgotten_when_reading_stops() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    assert_eq!(
        hotkeys.hotkey(CLICKING, &mut keyboard),
        [Fired(CLICKING), ModifiersReleased]
    );
    hotkeys.watch(Watching::default(), &mut keyboard);
    assert!(!keyboard.reading);
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    assert_eq!(
        press(&mut hotkeys, &mut keyboard, F6),
        [Fired(CLICKING), ModifiersReleased]
    );
}

#[test]
fn a_key_the_program_let_go_of_counts_as_up() {
    const ALT: u16 = 56;
    // The key state counted the program's Alt as the user's.
    let mut keyboard = holding(&[ALT]);
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    keyboard.down.clear();
    assert_eq!(hotkeys.own_release(ALT, &mut keyboard), []);
    assert_eq!(
        press(&mut hotkeys, &mut keyboard, F6),
        [Fired(CLICKING), ModifiersReleased]
    );
    // An owed release it held up comes with it.
    let mut keyboard = holding(&[ALT]);
    let mut hotkeys = registered();
    hotkeys.hotkey(CLICKING, &mut keyboard);
    keyboard.down.clear();
    let released = hotkeys.own_release(ALT, &mut keyboard);
    assert_eq!(after(released, &keyboard), (vec![ModifiersReleased], false));
}

#[test]
fn the_programs_release_leaves_a_key_the_user_pressed_down() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    hotkeys.key(CTRL, true, &mut keyboard);
    hotkeys.key(SHIFT, true, &mut keyboard);
    // The program lets go of a Ctrl of its own while the user holds theirs.
    assert_eq!(hotkeys.own_release(CTRL, &mut keyboard), []);
    assert_eq!(hotkeys.key(F6, true, &mut keyboard), [Fired(CLICKING)]);
}

#[test]
fn a_key_the_system_takes_for_a_stops_key_stops_it_from_raw_input() {
    // Keypad Enter is Enter to Windows: Ctrl+Shift+Enter fires on either.
    const ENTER: u16 = 28;
    const KEYPAD_ENTER: u16 = 96;
    let enter = Binding::parse("Ctrl+Shift+Enter").unwrap();
    let both = |binding: Binding| vec![binding.key(), Key::new(KEYPAD_ENTER).unwrap()];
    let mut hotkeys = Registered::new(vec![(CLICKING, enter)], both);
    let mut keyboard = FakeKeyboard::default();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    assert_eq!(
        press(&mut hotkeys, &mut keyboard, KEYPAD_ENTER),
        [Fired(CLICKING), ModifiersReleased]
    );
    assert_eq!(
        press(&mut hotkeys, &mut keyboard, ENTER),
        [Fired(CLICKING), ModifiersReleased]
    );
}

#[test]
fn letting_go_of_a_key_lets_go_of_its_seeded_aliases() {
    const ENTER: u16 = 28;
    const KEYPAD_ENTER: u16 = 96;
    let enter = Binding::parse("Ctrl+Shift+Enter").unwrap();
    let both = |binding: Binding| vec![binding.key(), Key::new(KEYPAD_ENTER).unwrap()];
    let mut hotkeys = Registered::new(vec![(CLICKING, enter)], both);
    // Enter held at the start: the key state shows both Enter keys down.
    let mut keyboard = holding(&[ENTER, KEYPAD_ENTER]);
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    keyboard.down.clear();
    hotkeys.key(ENTER, false, &mut keyboard);
    assert_eq!(
        press(&mut hotkeys, &mut keyboard, KEYPAD_ENTER),
        [Fired(CLICKING), ModifiersReleased]
    );
}

#[test]
fn letting_go_of_a_key_leaves_an_alias_whose_own_state_is_down() {
    // Y and Z: one hotkey's keys across two layouts, each with a state of
    // its own.
    const Y: u16 = 21;
    const Z: u16 = 44;
    let y = Binding::parse("Ctrl+Shift+Y").unwrap();
    let both = |binding: Binding| vec![binding.key(), Key::new(Z).unwrap()];
    let mut hotkeys = Registered::new(vec![(CLICKING, y)], both);
    let mut keyboard = holding(&[Y, Z]);
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    keyboard.down.retain(|&code| code != Z);
    hotkeys.key(Z, false, &mut keyboard);
    hotkeys.key(CTRL, true, &mut keyboard);
    hotkeys.key(SHIFT, true, &mut keyboard);
    // Y, still held since before, repeats: no press.
    assert_eq!(hotkeys.key(Y, true, &mut keyboard), []);
}

#[test]
fn the_programs_release_lets_go_of_its_seeded_aliases_too() {
    const ENTER: u16 = 28;
    const KEYPAD_ENTER: u16 = 96;
    let enter = Binding::parse("Ctrl+Shift+Enter").unwrap();
    let both = |binding: Binding| vec![binding.key(), Key::new(KEYPAD_ENTER).unwrap()];
    // The program's keypad Enter, which the key state shows as both, or
    // as Enter alone while the program's own key is left out.
    for seeded in [&[ENTER, KEYPAD_ENTER][..], &[ENTER]] {
        let mut hotkeys = Registered::new(vec![(CLICKING, enter)], both);
        let mut keyboard = holding(seeded);
        hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
        keyboard.down.clear();
        hotkeys.own_release(KEYPAD_ENTER, &mut keyboard);
        assert_eq!(
            press(&mut hotkeys, &mut keyboard, ENTER),
            [Fired(CLICKING), ModifiersReleased],
            "{seeded:?}"
        );
    }
}

#[test]
fn a_session_change_forgets_a_stop_key_let_go_of_on_another_desktop() {
    let mut keyboard = FakeKeyboard::default();
    let mut hotkeys = registered();
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    for code in [CTRL, SHIFT, F6] {
        hotkeys.key(code, true, &mut keyboard);
    }
    hotkeys.key(SHIFT, false, &mut keyboard);
    hotkeys.key(CTRL, false, &mut keyboard);
    // The run stopped; F6 comes up on the lock screen, unseen.
    hotkeys.watch(Watching::default(), &mut keyboard);
    assert!(
        keyboard.reading,
        "F6's raw press still waits for its hotkey"
    );
    assert_eq!(hotkeys.session_changed(&mut keyboard), []);
    assert!(!keyboard.reading);
    // The next run's first stop press counts.
    hotkeys.watch(stops(&[CLICKING]), &mut keyboard);
    assert_eq!(
        press(&mut hotkeys, &mut keyboard, F6),
        [Fired(CLICKING), ModifiersReleased]
    );
}

#[test]
fn a_session_change_during_the_modifier_wait_loses_it() {
    let mut keyboard = holding(&[CTRL, SHIFT]);
    let mut hotkeys = registered();
    hotkeys.hotkey(CLICKING, &mut keyboard);
    hotkeys.watch(WAITING, &mut keyboard);
    // UAC's desktop has the input: the key state reads every key up.
    keyboard.down.clear();
    let changed = hotkeys.session_changed(&mut keyboard);
    assert_eq!(after(changed, &keyboard), (vec![ReleaseLost], false));
}
