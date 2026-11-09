//! Binding parsing and matching, without devices.

use std::fmt::{Debug, Display};
use std::marker::PhantomData;

use super::*;
use crate::input::Key;

fn binding(text: &str) -> Binding {
    Binding::parse(text).unwrap_or_else(|err| panic!("{text:?}: {err}"))
}

fn key(name: &str) -> Key {
    Key::from_name(name).unwrap()
}

// Parsing

#[test]
fn bindings_parse_into_modifiers_and_a_key() {
    let ctrl_f6 = binding("Ctrl+F6");
    assert_eq!(ctrl_f6.key(), key("f6"));
    assert_eq!(ctrl_f6.modifiers(), [Modifier::Ctrl].into_iter().collect());
    let all = binding("Meta+Alt+Shift+Ctrl+Space");
    assert_eq!(all.modifiers(), Modifier::ALL.into_iter().collect());
    assert_eq!(all.key(), key("space"));
    let bare = binding("F13");
    assert_eq!(bare.modifiers(), Modifiers::NONE);
    assert_eq!(bare.key(), key("f13"));
}

#[test]
fn letter_case_does_not_matter() {
    for text in ["Ctrl+F6", "ctrl+f6", "CTRL+F6", "cTrL+f6"] {
        assert_eq!(binding(text), binding("Ctrl+F6"), "{text}");
    }
}

#[test]
fn bindings_show_in_one_form() {
    for (text, shown) in [
        ("ctrl+f6", "Ctrl+F6"),
        ("shift+CTRL+pageup", "Ctrl+Shift+Pageup"),
        ("META+alt+w", "Alt+Meta+W"),
        ("f24", "F24"),
        ("ctrl+1", "Ctrl+1"),
    ] {
        assert_eq!(binding(text).to_string(), shown, "{text}");
    }
}

#[test]
fn the_defaults_are_ctrl_f6_f7_f8() {
    let defaults = Bindings::default();
    assert_eq!(defaults.toggle_clicking, binding("Ctrl+F6"));
    assert_eq!(defaults.toggle_recording, binding("Ctrl+F7"));
    assert_eq!(defaults.toggle_playback, binding("Ctrl+F8"));
}

#[test]
fn invalid_bindings_say_why() {
    for (text, reason) in [
        ("", "empty part"),
        ("Ctrl+", "empty part"),
        ("+F6", "empty part"),
        ("Ctrl++F6", "empty part"),
        ("Ctrl + F6", "contains a space"),
        (" Ctrl+F6", "contains a space"),
        ("Ctrl+F6\t", "contains a space"),
        ("Hyper+F6", "unknown modifier \"Hyper\""),
        ("Super+F6", "unknown modifier \"Super\""),
        ("LeftCtrl+F6", "unknown modifier \"LeftCtrl\""),
        ("Ctrl+ctrl+F6", "names Ctrl twice"),
        ("Ctrl", "has no key"),
        ("Ctrl+Shift", "has no key"),
        ("Ctrl+F66", "unknown key \"F66\""),
        ("Ctrl+KEY_F6", "unknown key \"KEY_F6\""),
        ("Ctrl+leftctrl", "ends in a modifier key"),
        ("Shift+RightAlt", "ends in a modifier key"),
        ("leftmeta", "ends in a modifier key"),
        // The Kelvin sign lowercases to "k" in Unicode, not in ASCII.
        ("Ctrl+\u{212a}", "unknown key"),
    ] {
        let err = Binding::parse(text).expect_err(text);
        assert!(err.contains(reason), "{text:?}: {err}");
    }
}

#[test]
fn every_modifier_key_is_refused_as_the_key() {
    for code in Modifier::ALL.into_iter().flat_map(Modifier::codes) {
        let name = Key::new(code).and_then(Key::name).unwrap();
        assert!(Binding::parse(&format!("Ctrl+{name}")).is_err(), "{name}");
    }
}

// Choosing devices

#[test]
fn a_device_with_a_bound_key_or_a_modifier_is_read() {
    let bindings = Bindings::default();
    let cases: [(&str, &[u16], bool); 6] = [
        ("F6", &[64], true),
        ("F8", &[66], true),
        ("right Ctrl only", &[97], true),
        ("right Meta only", &[126], true),
        ("power button and mouse", &[116, 0x110], false),
        ("nothing", &[], false),
    ];
    for (what, codes, wanted) in cases {
        let read = bindings.wants_device(|code| codes.contains(&code));
        assert_eq!(read, wanted, "{what}");
    }
}

// Matching

const A: DeviceId = DeviceId(1);
const B: DeviceId = DeviceId(2);

const LEFTCTRL: u16 = 29;
const RIGHTCTRL: u16 = 97;
const LEFTSHIFT: u16 = 42;
const F6: u16 = 64;
const F7: u16 = 65;
const F9: u16 = 67;

/// One input to the matcher.
#[derive(Clone, Copy)]
enum Step {
    Key(DeviceId, u16, i32),
    Gone(DeviceId),
}

fn down(device: DeviceId, code: u16) -> Step {
    Step::Key(device, code, 1)
}

fn up(device: DeviceId, code: u16) -> Step {
    Step::Key(device, code, 0)
}

fn repeat(device: DeviceId, code: u16) -> Step {
    Step::Key(device, code, 2)
}

/// Feeds `steps` to `matcher` and returns what it sent.
fn feed(matcher: &mut Matcher, steps: &[Step]) -> Vec<HotkeyEvent> {
    steps
        .iter()
        .flat_map(|&step| {
            let reaction = match step {
                Step::Key(device, code, value) => {
                    matcher.key(device, KeyEvent::new(code, value).unwrap())
                }
                Step::Gone(device) => matcher.device_gone(device),
            };
            reaction.events()
        })
        .collect()
}

/// Feeds `steps` to a new matcher over `bindings` and returns what it sent.
fn run_with(bindings: Bindings, steps: &[Step]) -> Vec<HotkeyEvent> {
    feed(&mut Matcher::new(bindings), steps)
}

fn run(steps: &[Step]) -> Vec<HotkeyEvent> {
    run_with(Bindings::default(), steps)
}

use HotkeyEvent::{Fired, ModifiersReleased};

#[test]
fn ctrl_on_one_device_and_f6_on_another_fires() {
    assert_eq!(
        run(&[down(A, LEFTCTRL), down(B, F6), up(B, F6), up(A, LEFTCTRL)]),
        [Fired(Action::ToggleClicking), ModifiersReleased]
    );
}

#[test]
fn each_binding_fires_its_action() {
    let press = |code| run(&[down(A, LEFTCTRL), down(A, code)]);
    let fired = [F6, F7, 66, F9].map(press);
    assert_eq!(
        fired,
        [
            vec![Fired(Action::ToggleClicking)],
            vec![Fired(Action::ToggleRecording)],
            vec![Fired(Action::TogglePlayback)],
            vec![],
        ]
    );
}

#[test]
fn left_and_right_ctrl_are_the_same() {
    assert_eq!(
        run(&[down(A, RIGHTCTRL), down(A, F6)]),
        [Fired(Action::ToggleClicking)]
    );
}

#[test]
fn modifiers_must_match_exactly() {
    assert_eq!(run(&[down(A, F6)]), [], "no Ctrl");
    assert_eq!(
        run(&[down(A, LEFTCTRL), down(B, LEFTSHIFT), down(A, F6)]),
        [],
        "Ctrl+Shift"
    );
    assert_eq!(
        run(&[
            down(A, LEFTSHIFT),
            up(A, LEFTSHIFT),
            down(A, LEFTCTRL),
            down(A, F6)
        ]),
        [Fired(Action::ToggleClicking)],
        "Shift released first"
    );
}

#[test]
fn only_the_press_fires() {
    assert_eq!(
        run(&[
            down(A, LEFTCTRL),
            down(A, F6),
            repeat(A, F6),
            repeat(A, F6),
            up(A, F6)
        ]),
        [Fired(Action::ToggleClicking)]
    );
}

#[test]
fn a_key_pressed_before_ctrl_does_not_fire() {
    assert_eq!(
        run(&[down(A, F6), down(A, LEFTCTRL), repeat(A, F6), up(A, F6)]),
        []
    );
}

#[test]
fn ctrl_stays_held_while_either_side_or_device_holds_it() {
    assert_eq!(
        run(&[
            down(A, LEFTCTRL),
            down(B, RIGHTCTRL),
            up(A, LEFTCTRL),
            down(A, F6),
            up(B, RIGHTCTRL),
            down(A, F6),
        ]),
        [Fired(Action::ToggleClicking), ModifiersReleased]
    );
}

#[test]
fn a_release_without_a_press_changes_nothing() {
    assert_eq!(
        run(&[
            up(A, LEFTCTRL),
            down(B, LEFTCTRL),
            up(A, LEFTCTRL),
            down(B, F6)
        ]),
        [Fired(Action::ToggleClicking)]
    );
}

#[test]
fn the_release_comes_once_every_modifier_is_up() {
    assert_eq!(
        run(&[
            down(A, LEFTCTRL),
            down(A, F6),
            down(A, LEFTSHIFT),
            up(A, LEFTCTRL),
            up(A, LEFTSHIFT),
            down(A, LEFTCTRL),
            up(A, LEFTCTRL),
        ]),
        [Fired(Action::ToggleClicking), ModifiersReleased]
    );
}

#[test]
fn fires_before_the_release_share_one() {
    assert_eq!(
        run(&[down(A, LEFTCTRL), down(A, F6), down(A, F7), up(A, LEFTCTRL)]),
        [
            Fired(Action::ToggleClicking),
            Fired(Action::ToggleRecording),
            ModifiersReleased
        ]
    );
}

#[test]
fn a_binding_without_modifiers_is_released_at_once() {
    let bindings = Bindings {
        toggle_clicking: binding("F9"),
        ..Bindings::default()
    };
    assert_eq!(
        run_with(bindings, &[down(A, F9), up(A, F9)]),
        [Fired(Action::ToggleClicking), ModifiersReleased]
    );
}

#[test]
fn no_release_without_a_fire() {
    assert_eq!(run(&[down(A, LEFTCTRL), up(A, LEFTCTRL)]), []);
}

#[test]
fn a_device_that_goes_away_lets_go_of_its_keys() {
    let mut matcher = Matcher::new(Bindings::default());
    let sent = feed(
        &mut matcher,
        &[down(A, LEFTCTRL), down(B, F6), Step::Gone(A), down(B, F6)],
    );
    let expected = [Fired(Action::ToggleClicking), ModifiersReleased];
    assert_eq!(
        (sent, matcher.modifiers()),
        (expected.to_vec(), Modifiers::NONE)
    );
}

#[test]
fn another_device_going_away_keeps_the_wait() {
    let mut matcher = Matcher::new(Bindings::default());
    let sent = feed(
        &mut matcher,
        &[down(A, LEFTCTRL), down(A, F6), Step::Gone(B)],
    );
    let ctrl = [Modifier::Ctrl].into_iter().collect();
    let expected = vec![Fired(Action::ToggleClicking)];
    assert_eq!((sent, matcher.modifiers()), (expected, ctrl));
}

#[test]
fn only_evdev_key_values_make_events() {
    assert!(KeyEvent::new(F6, 3).is_none());
    assert!(KeyEvent::new(F6, -1).is_none());
}

// No key data can be shown

struct Probe<T>(PhantomData<T>);

trait Shows {
    fn shows(&self) -> bool {
        true
    }
}
impl<T: Debug> Shows for Probe<T> {}

trait ShowsNot {
    fn shows(&self) -> bool {
        false
    }
}
impl<T> ShowsNot for &Probe<T> {}

struct DisplayProbe<T>(PhantomData<T>);

trait Displays {
    fn displays(&self) -> bool {
        true
    }
}
impl<T: Display> Displays for DisplayProbe<T> {}

trait DisplaysNot {
    fn displays(&self) -> bool {
        false
    }
}
impl<T> DisplaysNot for &DisplayProbe<T> {}

#[test]
// The borrows choose the impl; clippy sees only that `String` needs none.
#[allow(clippy::needless_borrow)]
fn key_events_cannot_be_printed() {
    // Method lookup takes the `Debug` impl when it applies, and the fallback
    // on the reference otherwise. `String` checks the probes themselves.
    let strings = (
        (&Probe::<String>(PhantomData)).shows(),
        (&DisplayProbe::<String>(PhantomData)).displays(),
    );
    let key_events = (
        (&Probe::<KeyEvent>(PhantomData)).shows(),
        (&DisplayProbe::<KeyEvent>(PhantomData)).displays(),
    );
    assert_eq!((strings, key_events), ((true, true), (false, false)));
}

#[test]
fn what_the_controller_gets_names_no_key() {
    let mut matcher = Matcher::new(Bindings::default());
    let mut shown = String::new();
    for code in 1..=248 {
        for value in [1, 2, 0] {
            let reaction = matcher.key(A, KeyEvent::new(code, value).unwrap());
            shown.push_str(&format!("{reaction:?}"));
        }
    }
    assert!(!shown.chars().any(|c| c.is_ascii_digit()), "{shown}");
}
