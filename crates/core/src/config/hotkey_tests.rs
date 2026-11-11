//! Parsing and validating `[hotkeys]`.

use super::*;
use crate::hotkeys::{Binding, Bindings};
use crate::input::Key;

fn parse(text: &str) -> Config {
    Config::parse(text).unwrap_or_else(|err| panic!("{text:?}: {err:?}"))
}

/// The line and reason of an invalid file.
fn invalid(text: &str) -> (Option<usize>, String) {
    match Config::parse(text) {
        Err(Problem::Invalid { line, reason }) => (line, reason),
        other => panic!("{text:?} should be invalid, got {other:?}"),
    }
}

/// The bindings a `[hotkeys]` section gives.
fn hotkeys(section: &str) -> Bindings {
    parse(&format!("version = 1\n[hotkeys]\n{section}\n")).hotkeys
}

/// The line and reason of an invalid file with these `[clicker]` and
/// `[hotkeys]` sections; the hotkeys start on line 4.
fn invalid_hotkeys(clicker: &str, hotkeys: &str) -> (Option<usize>, String) {
    invalid(&format!(
        "version = 1\n[clicker]\n{clicker}\n[hotkeys]\n{hotkeys}\n"
    ))
}

#[test]
fn hotkeys_default_to_ctrl_f6_f7_f8() {
    assert_eq!(parse("version = 1\n").hotkeys, Bindings::default());
    assert_eq!(hotkeys(""), Bindings::default());
}

#[test]
fn each_hotkey_is_set_on_its_own() {
    let defaults = Bindings::default();
    let set = hotkeys("toggle_recording = \"ctrl+F10\"");
    assert_eq!(set.toggle_clicking, defaults.toggle_clicking);
    assert_eq!(set.toggle_recording, Binding::parse("Ctrl+F10").unwrap());
    assert_eq!(set.toggle_playback, defaults.toggle_playback);
}

#[test]
fn a_bad_hotkey_is_reported_at_its_line() {
    let (line, reason) = invalid_hotkeys(
        "",
        "toggle_clicking = \"Ctrl+F6\"\ntoggle_playback = \"Ctrl+Hyper\"",
    );
    assert_eq!(line, Some(6), "{reason}");
    assert!(reason.contains("unknown key \"Hyper\""), "{reason}");
    let (line, reason) = invalid("version = 1\n[hotkeys]\ntoggle_clicking = 6\n");
    assert_eq!(line, Some(3), "{reason}");
}

#[test]
fn two_actions_on_one_hotkey_are_rejected_where_written() {
    let both = invalid_hotkeys("", "toggle_clicking = \"F9\"\ntoggle_playback = \"f9\"");
    assert_eq!(both.0, Some(6), "the later one: {}", both.1);
    assert!(
        both.1.contains("hotkey F9 is bound to two actions"),
        "{}",
        both.1
    );
    // The playback default, Ctrl+Shift+F8, taken by the action before it.
    let earlier = invalid_hotkeys("", "toggle_clicking = \"Ctrl+Shift+F8\"");
    assert_eq!(earlier.0, Some(5), "{}", earlier.1);
    // The clicking default, Ctrl+Shift+F6, taken by an action after it.
    let later = invalid_hotkeys("", "\ntoggle_playback = \"ctrl+shift+f6\"");
    assert_eq!(later.0, Some(6), "{}", later.1);
    assert!(later.1.contains("Ctrl+Shift+F6"), "{}", later.1);
}

#[test]
fn a_hotkey_on_the_key_the_clicker_presses_is_rejected() {
    for (mode, hotkey) in [("key_repeat", "F9"), ("hold_key", "Ctrl+f9")] {
        let (line, reason) = invalid_hotkeys(
            &format!("mode = \"{mode}\"\nkey = \"f9\""),
            &format!("toggle_recording = \"{hotkey}\""),
        );
        assert_eq!(line, Some(6), "{mode}: {reason}");
        assert!(reason.contains("the key the clicker presses"), "{reason}");
    }
}

#[test]
fn a_config_binding_on_the_clicker_key_is_fine_where_the_desktop_owns_the_hotkeys() {
    // The desktop's triggers may differ from the config's bindings, the
    // defaults included; the controller checks the real ones.
    for hotkeys in ["", "[hotkeys]\ntoggle_recording = \"Ctrl+F9\"\n"] {
        for key in ["f6", "f9"] {
            let text = format!(
                "version = 1\n[clicker]\nmode = \"hold_key\"\nkey = \"{key}\"\n{hotkeys}[input]\nhotkeys = \"portal\"\n"
            );
            let config = parse(&text);
            assert_eq!(config.input.hotkeys, HotkeyRoute::Portal, "{text}");
            assert_eq!(config.clicker.key, Key::from_name(key).unwrap());
        }
    }
    // On the kernel route, and the automatic one that may pick it, the same
    // file is rejected: a default binding.
    for route in ["kernel", "auto"] {
        let (line, reason) = invalid(&format!(
            "version = 1\n[clicker]\nmode = \"hold_key\"\nkey = \"f6\"\n[input]\nhotkeys = \"{route}\"\n"
        ));
        assert_eq!(line, None, "{route}: {reason}");
        assert!(reason.contains("the key the clicker presses"), "{reason}");
    }
}

#[test]
fn a_hotkey_on_the_clicker_key_is_fine_in_a_mouse_mode_or_on_another_key() {
    let parsed = |clicker: &str| {
        let text =
            format!("version = 1\n[clicker]\n{clicker}\n[hotkeys]\ntoggle_clicking = \"F9\"\n");
        Config::parse(&text).is_ok()
    };
    let fine = [
        parsed("mode = \"click\"\nkey = \"f9\""),
        parsed("mode = \"hold_key\"\nkey = \"f10\""),
    ];
    assert_eq!(fine, [true, true]);
}
