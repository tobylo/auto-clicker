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

#[test]
fn output_defaults_to_the_kernel_and_reads_portal() {
    let outputs = [
        parse("version = 1\n").input.output,
        parse("version = 1\n[input]\n").input.output,
        parse("version = 1\n[input]\noutput = \"kernel\"\n")
            .input
            .output,
        parse("version = 1\n[input]\noutput = \"portal\"\n")
            .input
            .output,
    ];
    assert_eq!(
        outputs,
        [
            OutputRoute::Kernel,
            OutputRoute::Kernel,
            OutputRoute::Kernel,
            OutputRoute::Portal
        ]
    );
}

#[test]
fn hotkeys_default_to_the_kernel_and_read_portal() {
    let routes = [
        parse("version = 1\n").input.hotkeys,
        parse("version = 1\n[input]\noutput = \"portal\"\n")
            .input
            .hotkeys,
        parse("version = 1\n[input]\nhotkeys = \"kernel\"\n")
            .input
            .hotkeys,
        parse("version = 1\n[input]\nhotkeys = \"portal\"\n")
            .input
            .hotkeys,
    ];
    assert_eq!(
        routes,
        [
            HotkeyRoute::Kernel,
            HotkeyRoute::Kernel,
            HotkeyRoute::Kernel,
            HotkeyRoute::Portal
        ]
    );
    // The routes are chosen separately.
    let mixed = parse("version = 1\n[input]\nhotkeys = \"portal\"\n").input;
    assert_eq!(mixed.output, OutputRoute::Kernel);
}

#[test]
fn an_unknown_hotkey_route_is_rejected_at_its_line() {
    for value in ["evdev", "Portal", "auto"] {
        let (line, reason) = invalid(&format!("version = 1\n[input]\nhotkeys = \"{value}\"\n"));
        assert_eq!(line, Some(3), "{value}: {reason}");
        assert!(
            reason.contains("kernel") && reason.contains("portal"),
            "{value}: {reason}"
        );
    }
}

#[test]
fn an_unknown_output_or_input_key_is_rejected_at_its_line() {
    let (line, reason) = invalid("version = 1\n[input]\noutput = \"Portal\"\n");
    assert_eq!(line, Some(3), "{reason}");
    assert!(
        reason.contains("kernel") && reason.contains("portal"),
        "{reason}"
    );
    let (line, reason) = invalid("version = 1\n[input]\nbackend = \"portal\"\n");
    assert_eq!(line, Some(3), "{reason}");
}
