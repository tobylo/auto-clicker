//! `[input]`: the routes output and hotkeys take.

use super::*;

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

/// The route each spelling of `key` gives, through `pick`.
fn routes<R>(key: &str, pick: impl Fn(&InputSettings) -> R) -> Vec<R> {
    [
        "version = 1\n".to_owned(),
        "version = 1\n[input]\n".to_owned(),
        format!("version = 1\n[input]\n{key} = \"auto\"\n"),
        format!("version = 1\n[input]\n{key} = \"kernel\"\n"),
        format!("version = 1\n[input]\n{key} = \"portal\"\n"),
    ]
    .iter()
    .map(|text| pick(&parse(text).input))
    .collect()
}

#[test]
fn output_defaults_to_auto_and_reads_each_route() {
    use OutputRoute::{Auto, Kernel, Portal};
    let outputs = routes("output", |input| input.output);
    assert_eq!(outputs, [Auto, Auto, Auto, Kernel, Portal]);
}

#[test]
fn hotkeys_default_to_auto_and_read_each_route() {
    use HotkeyRoute::{Auto, Kernel, Portal};
    let hotkeys = routes("hotkeys", |input| input.hotkeys);
    assert_eq!(hotkeys, [Auto, Auto, Auto, Kernel, Portal]);
    // The routes are chosen separately, each way round.
    let mixed = parse("version = 1\n[input]\nhotkeys = \"portal\"\n").input;
    assert_eq!(mixed.output, OutputRoute::Auto);
    let mixed = parse("version = 1\n[input]\noutput = \"portal\"\n").input;
    assert_eq!(mixed.hotkeys, HotkeyRoute::Auto);
}

#[test]
fn every_route_round_trips_through_the_canonical_writer() {
    let outputs = [OutputRoute::Auto, OutputRoute::Kernel, OutputRoute::Portal];
    let hotkeys = [HotkeyRoute::Auto, HotkeyRoute::Kernel, HotkeyRoute::Portal];
    for output in outputs {
        for hotkeys in hotkeys {
            let config = Config {
                input: InputSettings { output, hotkeys },
                ..Config::default()
            };
            let text = render(&config);
            assert_eq!(parse(&text), config, "{text}");
            let draft = Draft::from(&config);
            assert_eq!(draft.to_config(), Ok(config), "{output:?} {hotkeys:?}");
        }
    }
}

#[test]
fn only_the_portal_route_leaves_the_hotkeys_to_the_desktop() {
    assert!(HotkeyRoute::Auto.may_read_keyboards());
    assert!(HotkeyRoute::Kernel.may_read_keyboards());
    assert!(!HotkeyRoute::Portal.may_read_keyboards());
}

#[test]
fn an_unknown_hotkey_route_is_rejected_at_its_line() {
    for value in ["evdev", "Portal", "Auto", "automatic"] {
        let (line, reason) = invalid(&format!("version = 1\n[input]\nhotkeys = \"{value}\"\n"));
        assert_eq!(line, Some(3), "{value}: {reason}");
        assert!(
            ["auto", "kernel", "portal"]
                .iter()
                .all(|route| reason.contains(route)),
            "{value}: {reason}"
        );
    }
}

#[test]
fn an_unknown_output_or_input_key_is_rejected_at_its_line() {
    let (line, reason) = invalid("version = 1\n[input]\noutput = \"Portal\"\n");
    assert_eq!(line, Some(3), "{reason}");
    assert!(
        ["auto", "kernel", "portal"]
            .iter()
            .all(|route| reason.contains(route)),
        "{reason}"
    );
    let (line, reason) = invalid("version = 1\n[input]\nbackend = \"portal\"\n");
    assert_eq!(line, Some(3), "{reason}");
}
