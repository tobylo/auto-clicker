//! Binding parsing and matching, without devices.

use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::time::Duration;

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
        ("ctrl+102nd", "Ctrl+102nd"),
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

/// One input to the matcher. Times are in milliseconds.
#[derive(Clone, Copy)]
enum Step {
    /// A key event at a time: [`LATE`] unless [`at`] sets an earlier one.
    Key(DeviceId, u16, i32, u64),
    /// The device was unplugged or failed at a time.
    Gone(DeviceId, u64),
    /// The device's events were lost.
    Lost(DeviceId),
    /// The device was read again after lost events at a time, holding these
    /// keys.
    Read(DeviceId, &'static [u16], u64),
}

/// Later than every out-of-order change in these tests.
const LATE: u64 = 1_000_000;

fn down(device: DeviceId, code: u16) -> Step {
    Step::Key(device, code, 1, LATE)
}

fn up(device: DeviceId, code: u16) -> Step {
    Step::Key(device, code, 0, LATE)
}

fn repeat(device: DeviceId, code: u16) -> Step {
    Step::Key(device, code, 2, LATE)
}

/// A key event that happened at `ms`, though it is read later.
fn at(ms: u64, step: Step) -> Step {
    match step {
        Step::Key(device, code, value, _) => Step::Key(device, code, value, ms),
        other => other,
    }
}

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

/// Feeds `steps` to `matcher` and returns what it sent.
fn feed(matcher: &mut Matcher, steps: &[Step]) -> Vec<HotkeyEvent> {
    steps
        .iter()
        .flat_map(|&step| {
            let reaction = match step {
                Step::Key(device, code, value, time) => {
                    matcher.key(device, KeyEvent::new(code, value, ms(time)).unwrap())
                }
                Step::Gone(device, now) => matcher.device_lost(device, ms(now)),
                Step::Lost(device) => matcher.events_lost(device),
                Step::Read(device, held, now) => {
                    matcher.resync(device, |code| held.contains(&code), ms(now));
                    Reaction::default()
                }
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

use HotkeyEvent::{Fired, ModifiersReleased, ReleaseLost};

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
fn every_modifier_key_counts_as_its_modifier() {
    assert_eq!(
        matcher::MODIFIER_CODES.to_vec(),
        Modifier::ALL
            .into_iter()
            .flat_map(Modifier::codes)
            .collect::<Vec<_>>()
    );
    for modifier in Modifier::ALL {
        let bindings = Bindings {
            toggle_clicking: binding(&format!("{}+F9", modifier.name())),
            ..Bindings::default()
        };
        for code in modifier.codes() {
            let sent = run_with(bindings, &[down(A, code), down(A, F9)]);
            assert_eq!(sent, [Fired(Action::ToggleClicking)], "{code}");
        }
    }
}

#[test]
fn a_modifier_held_twice_is_held_once() {
    let held_twice = [
        [down(A, LEFTCTRL), down(A, RIGHTCTRL)],
        [down(A, LEFTCTRL), down(B, LEFTCTRL)],
        [down(A, LEFTCTRL), down(A, LEFTCTRL)],
    ];
    for steps in held_twice {
        let sent = run(&[steps[0], steps[1], down(A, F6)]);
        assert_eq!(sent, [Fired(Action::ToggleClicking)]);
    }
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
fn a_release_never_seen_pressed_leaves_nothing_held() {
    let bindings = Bindings {
        toggle_clicking: binding("F9"),
        ..Bindings::default()
    };
    assert_eq!(
        run_with(bindings, &[up(A, LEFTCTRL), down(A, F9)]),
        [Fired(Action::ToggleClicking), ModifiersReleased]
    );
}

#[test]
fn nothing_fires_while_a_device_that_lost_events_is_not_read_again() {
    let sent = run(&[
        Step::Lost(A),
        down(B, LEFTCTRL),
        down(B, F6),
        Step::Read(A, &[], 500),
        down(B, F6),
    ]);
    assert_eq!(sent, [Fired(Action::ToggleClicking)]);
}

#[test]
fn a_press_older_than_a_read_after_lost_events_never_fires() {
    // A's read finds nothing held, but B's Ctrl+F6 queued from before it
    // may have been pressed with a modifier A held then.
    let sent = run(&[
        Step::Lost(A),
        Step::Read(A, &[], 500),
        at(400, down(B, LEFTCTRL)),
        at(450, down(B, F6)),
        down(B, F6),
    ]);
    assert_eq!(sent, [Fired(Action::ToggleClicking)]);
}

#[test]
fn a_press_older_than_an_unplug_never_fires() {
    // Ctrl+Shift+F6 pressed; B, holding Shift, is unplugged before A's F6
    // is read.
    let sent = run(&[
        down(A, LEFTCTRL),
        down(B, LEFTSHIFT),
        Step::Gone(B, 500),
        at(300, down(A, F6)),
        down(A, F6),
    ]);
    assert_eq!(sent, [Fired(Action::ToggleClicking)]);
}

#[test]
fn queued_events_older_than_a_read_still_change_the_state() {
    // The read at 500 saw Ctrl held: the queued up at 400 and down at 450
    // replay to the same state.
    let sent = run(&[
        Step::Lost(A),
        Step::Read(A, &[LEFTCTRL], 500),
        at(400, up(A, LEFTCTRL)),
        at(450, down(A, LEFTCTRL)),
        down(A, F6),
    ]);
    assert_eq!(sent, [Fired(Action::ToggleClicking)]);
}

#[test]
fn a_read_replaces_what_the_device_held() {
    let mut matcher = Matcher::new(Bindings::default());
    feed(
        &mut matcher,
        &[
            down(A, LEFTCTRL),
            Step::Lost(A),
            Step::Read(A, &[LEFTSHIFT], 500),
        ],
    );
    let shift = [Modifier::Shift].into_iter().collect();
    assert_eq!(matcher.modifiers(), shift);
}

#[test]
fn lost_events_lose_an_awaited_release_and_a_read_never_claims_one() {
    let sent = run(&[
        down(A, LEFTCTRL),
        down(A, F6),
        Step::Lost(A),
        Step::Read(A, &[], 500),
        down(B, LEFTSHIFT),
        up(B, LEFTSHIFT),
    ]);
    assert_eq!(sent, [Fired(Action::ToggleClicking), ReleaseLost]);
}

#[test]
fn a_device_lost_holding_a_modifier_loses_the_release() {
    let mut matcher = Matcher::new(Bindings::default());
    let sent = feed(
        &mut matcher,
        &[down(A, LEFTCTRL), down(A, F6), Step::Gone(A, 500)],
    );
    // Another keyboard's modifiers cannot complete it any more.
    let after = feed(
        &mut matcher,
        &[down(B, LEFTSHIFT), up(B, LEFTSHIFT), down(B, F6)],
    );
    assert_eq!(
        (sent, matcher.modifiers(), after),
        (
            vec![Fired(Action::ToggleClicking), ReleaseLost],
            Modifiers::NONE,
            vec![]
        )
    );
}

#[test]
fn a_device_lost_holding_nothing_keeps_the_wait() {
    let sent = run(&[
        down(A, LEFTCTRL),
        down(A, F6),
        Step::Gone(B, 500),
        up(A, LEFTCTRL),
    ]);
    assert_eq!(sent, [Fired(Action::ToggleClicking), ModifiersReleased]);
}

#[test]
fn a_device_gone_before_it_is_read_again_is_not_waited_for() {
    let sent = run(&[
        Step::Lost(A),
        Step::Gone(A, 500),
        down(B, LEFTCTRL),
        down(B, F6),
    ]);
    assert_eq!(sent, [Fired(Action::ToggleClicking)]);
}

#[test]
fn nothing_is_lost_without_a_wait() {
    let sent = run(&[down(A, LEFTCTRL), Step::Gone(A, 500), Step::Lost(B)]);
    assert_eq!(sent, []);
}

#[test]
fn no_release_without_a_fire() {
    assert_eq!(run(&[down(A, LEFTCTRL), up(A, LEFTCTRL)]), []);
}

#[test]
fn only_evdev_key_values_make_events() {
    assert!(KeyEvent::new(F6, 3, ms(1)).is_none());
    assert!(KeyEvent::new(F6, -1, ms(1)).is_none());
}

#[test]
fn unavailable_hotkeys_say_why() {
    let messages = [
        Unavailable::NoKeyboard,
        Unavailable::NoPermission,
        Unavailable::WatchFailed("no such directory".to_owned()),
    ]
    .map(|why| why.to_string());
    assert_eq!(
        messages,
        [
            "hotkeys are unavailable: no keyboard found",
            "hotkeys are unavailable: keyboards cannot be read; add your user to the input group and log in again",
            "hotkeys stopped: watching for keyboards failed: no such directory",
        ]
    );
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
            let reaction = matcher.key(A, KeyEvent::new(code, value, ms(1)).unwrap());
            shown.push_str(&format!("{reaction:?}"));
        }
    }
    assert!(!shown.chars().any(|c| c.is_ascii_digit()), "{shown}");
}
