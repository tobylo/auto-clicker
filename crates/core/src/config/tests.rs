//! Parsing, validating and loading the config file.

use std::fs;
use std::num::NonZeroU32;
use std::time::Duration;

use super::*;
use crate::clicker::{INTERVAL_FLOOR, INTERVAL_MAX, Interval, Jitter, Mode, Position, Repeat};
use crate::hotkeys::{Binding, Bindings};
use crate::input::{AbsolutePosition, Button, Key};
use crate::test_support::{TempDir, permissions_apply};

fn parse(text: &str) -> Config {
    Config::parse(text).unwrap_or_else(|err| panic!("{text:?}: {err:?}"))
}

/// The config `[clicker]` section `clicker` gives.
fn clicker(section: &str) -> ClickerSettings {
    parse(&format!("version = 1\n[clicker]\n{section}\n")).clicker
}

/// The line and reason of an invalid file.
fn invalid(text: &str) -> (Option<usize>, String) {
    match Config::parse(text) {
        Err(Problem::Invalid { line, reason }) => (line, reason),
        other => panic!("{text:?} should be invalid, got {other:?}"),
    }
}

/// The reason a `[clicker]` section is invalid; it is always on line 3.
fn invalid_clicker(section: &str) -> String {
    let (line, reason) = invalid(&format!("version = 1\n[clicker]\n{section}\n"));
    assert_eq!(line, Some(3), "{section:?}: {reason}");
    reason
}

fn ms(n: u64) -> Interval {
    Interval::new(Duration::from_millis(n)).unwrap()
}

// Valid files

#[test]
fn a_file_with_only_the_version_gives_the_defaults() {
    assert_eq!(parse("version = 1\n"), Config::default());
    assert_eq!(parse("version = 1\n[clicker]\n"), Config::default());
}

#[test]
fn defaults_wait_five_seconds_before_a_tray_start() {
    assert_eq!(Config::default().tray_start_delay, Duration::from_secs(5));
}

#[test]
fn every_option_is_read() {
    let config = parse(
        r#"
        version = 1
        tray_start_delay = 0

        [clicker]
        mode = "double_click"
        interval_ms = 250
        jitter_percent = 50
        button = "right"
        key = "f6"
        position = { x = 0, y = 65535 }
        repeat = 7

        [hotkeys]
        toggle_clicking = "Ctrl+Shift+F9"
        toggle_recording = "alt+r"
        toggle_playback = "F24"

        [input]
        output = "portal"
        "#,
    );
    let expected = Config {
        tray_start_delay: Duration::ZERO,
        hotkeys: Bindings {
            toggle_clicking: Binding::parse("Ctrl+Shift+F9").unwrap(),
            toggle_recording: Binding::parse("Alt+R").unwrap(),
            toggle_playback: Binding::parse("F24").unwrap(),
        },
        clicker: ClickerSettings {
            mode: Mode::DoubleClick,
            interval: ms(250),
            jitter: Jitter::new(50).unwrap(),
            button: Button::Right,
            key: Key::from_name("f6").unwrap(),
            position: Position::Fixed(AbsolutePosition { x: 0, y: 65535 }),
            repeat: Repeat::Times(NonZeroU32::new(7).unwrap()),
        },
        input: InputSettings {
            output: OutputRoute::Portal,
        },
    };
    assert_eq!(config, expected);
}

#[test]
fn every_mode_and_button_has_its_name() {
    let modes = [
        ("click", Mode::Click),
        ("double_click", Mode::DoubleClick),
        ("hold_button", Mode::HoldButton),
        ("key_repeat", Mode::KeyRepeat),
        ("hold_key", Mode::HoldKey),
    ];
    for (name, mode) in modes {
        assert_eq!(clicker(&format!("mode = {name:?}")).mode, mode);
    }
    let buttons = [
        ("left", Button::Left),
        ("right", Button::Right),
        ("middle", Button::Middle),
        ("back", Button::Side),
        ("forward", Button::Extra),
    ];
    for (name, button) in buttons {
        assert_eq!(clicker(&format!("button = {name:?}")).button, button);
    }
}

#[test]
fn clicks_per_second_sets_the_interval() {
    assert_eq!(clicker("clicks_per_second = 10").interval, ms(100));
    assert_eq!(clicker("clicks_per_second = 10.0").interval, ms(100));
    assert_eq!(clicker("clicks_per_second = 2.5").interval, ms(400));
}

#[test]
fn the_interval_floor_itself_is_allowed() {
    let floor_ms = INTERVAL_FLOOR.as_millis();
    assert_eq!(
        clicker(&format!("interval_ms = {floor_ms}")).interval.get(),
        INTERVAL_FLOOR
    );
    let double = clicker(&format!(
        "mode = \"double_click\"\ninterval_ms = {}",
        floor_ms * 2
    ));
    assert_eq!(double.interval.get(), INTERVAL_FLOOR * 2);
}

#[test]
fn a_key_is_a_name_or_a_code() {
    assert_eq!(clicker("key = \"w\"").key, Key::from_name("w").unwrap());
    assert_eq!(clicker("key = 84").key, Key::new(84).unwrap());
    // The extra key beside the left Shift on ISO keyboards.
    assert_eq!(clicker("key = \"102nd\"").key, Key::new(86).unwrap());
}

#[test]
fn position_and_repeat_take_their_words() {
    assert_eq!(clicker("position = \"cursor\"").position, Position::Cursor);
    assert_eq!(
        clicker("repeat = \"until_stopped\"").repeat,
        Repeat::UntilStopped
    );
    assert_eq!(
        clicker("[clicker.position]\nx = 1\ny = 2").position,
        Position::Fixed(AbsolutePosition { x: 1, y: 2 })
    );
}

// Invalid files

#[test]
fn the_version_is_required_and_must_be_one() {
    let (_, missing) = invalid("[clicker]\n");
    assert!(missing.contains("version"), "{missing}");
    assert_eq!(
        invalid("# comment\nversion = 2\n"),
        (
            Some(2),
            "unsupported config version 2; this program reads version 1".to_owned()
        )
    );
}

#[test]
fn unknown_keys_are_rejected_in_every_table() {
    let cases = [
        ("version = 1\ncolour = \"red\"\n", "colour"),
        (
            "version = 1\n[clicker]\nintervall_ms = 100\n",
            "intervall_ms",
        ),
        (
            "version = 1\n[clicker]\nposition = { x = 1, y = 2, z = 3 }\n",
            "`z`",
        ),
        (
            "version = 1\n[hotkeys]\ntoggle_clicks = \"Ctrl+F6\"\n",
            "toggle_clicks",
        ),
    ];
    for (text, key) in cases {
        let (line, reason) = invalid(text);
        let named = reason.contains(key);
        assert_eq!(
            (line, named),
            (Some(text.lines().count()), true),
            "{reason}"
        );
    }
}

#[test]
fn an_interval_below_the_floor_names_its_field() {
    let cases = [
        ("interval_ms = 5", "interval_ms must be from 10 to 86400000"),
        (
            "clicks_per_second = 200",
            "clicks_per_second must be from 0.000012 to 100",
        ),
    ];
    for (section, reason) in cases {
        assert_eq!(invalid_clicker(section), reason);
    }
}

#[test]
fn the_top_rate_is_allowed() {
    let rates = [
        clicker("clicks_per_second = 100").interval.get(),
        clicker("mode = \"double_click\"\nclicks_per_second = 50")
            .interval
            .get(),
    ];
    assert_eq!(rates, [INTERVAL_FLOOR, INTERVAL_FLOOR * 2]);
}

#[test]
fn the_longest_interval_is_a_day() {
    assert_eq!(
        clicker("interval_ms = 86400000").interval.get(),
        INTERVAL_MAX
    );
    assert_eq!(INTERVAL_MAX, Duration::from_secs(86_400));
}

#[test]
fn a_double_click_needs_twice_the_floor() {
    // The mode is on line 3; the error points at the rate or interval, on line 4.
    let suffix = " for a double click, which fits two clicks in each interval";
    let cases = [
        (
            "interval_ms = 15",
            "interval_ms must be from 20 to 86400000",
        ),
        (
            "clicks_per_second = 60",
            "clicks_per_second must be from 0.000012 to 50",
        ),
    ];
    for (setting, reason) in cases {
        let text = format!("version = 1\n[clicker]\nmode = \"double_click\"\n{setting}\n");
        assert_eq!(invalid(&text), (Some(4), format!("{reason}{suffix}")));
    }
}

#[test]
fn bad_intervals_are_rejected() {
    for section in [
        "interval_ms = 0",
        "interval_ms = -100",
        "interval_ms = 86400001",
        "interval_ms = 9223372036854775807",
    ] {
        let reason = invalid_clicker(section);
        assert!(
            reason.starts_with("interval_ms must be"),
            "{section}: {reason}"
        );
    }
    for section in [
        "clicks_per_second = 0",
        "clicks_per_second = -1",
        "clicks_per_second = inf",
        "clicks_per_second = nan",
        "clicks_per_second = 1e-9",
    ] {
        let reason = invalid_clicker(section);
        assert!(
            reason.starts_with("clicks_per_second must be"),
            "{section}: {reason}"
        );
    }
}

#[test]
fn an_interval_and_a_rate_together_are_rejected() {
    let (line, reason) =
        invalid("version = 1\n[clicker]\ninterval_ms = 100\nclicks_per_second = 10\n");
    assert_eq!(line, Some(4));
    assert_eq!(
        reason,
        "set either interval_ms or clicks_per_second, not both"
    );
}

#[test]
fn jitter_is_from_zero_to_fifty() {
    assert_eq!(
        clicker("jitter_percent = 0").jitter,
        Jitter::new(0).unwrap()
    );
    for section in [
        "jitter_percent = 51",
        "jitter_percent = -1",
        "jitter_percent = 300",
    ] {
        assert_eq!(
            invalid_clicker(section),
            "jitter_percent must be from 0 to 50"
        );
    }
}

#[test]
fn bad_names_say_what_is_allowed() {
    let reason = invalid_clicker("mode = \"triple_click\"");
    assert!(reason.contains("double_click"), "{reason}");
    let reason = invalid_clicker("button = \"Left\"");
    assert!(reason.contains("`left`"), "{reason}");
    for section in ["key = \"Space\"", "key = \"KEY_SPACE\"", "key = \"ctrl\""] {
        let reason = invalid_clicker(section);
        assert!(
            reason.starts_with("unknown key name"),
            "{section}: {reason}"
        );
    }
    for section in ["key = 0", "key = 249", "key = -1"] {
        let reason = invalid_clicker(section);
        assert!(
            reason.contains("is not from 1 to 248"),
            "{section}: {reason}"
        );
    }
    let reason = invalid_clicker("key = true");
    assert!(reason.contains("key name"), "{reason}");
}

#[test]
fn bad_positions_say_what_is_allowed() {
    let reason = invalid_clicker("position = \"center\"");
    assert!(reason.contains("\"cursor\""), "{reason}");
    let reason = invalid_clicker("position = { x = 65536, y = 0 }");
    assert!(reason.contains("65535"), "{reason}");
    let reason = invalid_clicker("position = { x = -1, y = 0 }");
    assert!(reason.contains("65535"), "{reason}");
    let reason = invalid_clicker("position = { x = 1 }");
    assert!(reason.contains("`y`"), "{reason}");
}

#[test]
fn bad_repeats_say_what_is_allowed() {
    for section in ["repeat = 0", "repeat = -3", "repeat = 4294967296"] {
        let reason = invalid_clicker(section);
        assert!(reason.contains("\"until_stopped\""), "{section}: {reason}");
    }
    let reason = invalid_clicker("repeat = \"forever\"");
    assert!(reason.contains("until_stopped"), "{reason}");
}

#[test]
fn the_tray_start_delay_is_from_zero_to_an_hour() {
    assert_eq!(
        parse("version = 1\ntray_start_delay = 3600\n").tray_start_delay,
        Duration::from_secs(3600)
    );
    for value in ["-1", "3601"] {
        let text = format!("version = 1\ntray_start_delay = {value}\n");
        let expected = "tray_start_delay must be from 0 to 3600 seconds".to_owned();
        assert_eq!(invalid(&text), (Some(2), expected));
    }
    for value in ["1.5", "\"5\""] {
        let (line, reason) = invalid(&format!("version = 1\ntray_start_delay = {value}\n"));
        assert_eq!(line, Some(2), "{value}: {reason}");
    }
}

#[test]
fn malformed_toml_reports_its_line() {
    let (line, _) = invalid("version = 1\n[clicker]\ninterval_ms = = 3\n");
    assert_eq!(line, Some(3));
}

// Loading

#[test]
fn a_missing_file_gives_the_defaults_and_writes_nothing() {
    let dir = TempDir::new();
    let loaded = Config::load(&dir.path().join("config.toml"));

    assert_eq!(loaded.config, Config::default());
    assert!(loaded.error.is_none());
    assert_eq!(dir.entries(), Vec::<String>::new());
}

#[test]
fn a_valid_file_is_loaded() {
    let dir = TempDir::new();
    let path = dir.path().join("config.toml");
    fs::write(&path, "version = 1\n[clicker]\nbutton = \"middle\"\n").unwrap();

    let loaded = Config::load(&path);

    assert!(loaded.error.is_none());
    assert_eq!(loaded.config.clicker.button, Button::Middle);
}

#[test]
fn an_invalid_file_gives_the_defaults_reports_it_and_is_left_alone() {
    let dir = TempDir::new();
    let path = dir.path().join("config.toml");
    let text = b"version = 1\n[clicker]\ninterval_ms = 1\n";
    fs::write(&path, text).unwrap();

    let loaded = Config::load(&path);

    let message = loaded
        .error
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let reported = message.starts_with(&format!("{}:3: interval_ms must be", path.display()));
    assert!(reported, "{message}");
    let left_alone = (fs::read(&path).unwrap(), dir.entries());
    let expected = (text.to_vec(), vec!["config.toml".to_owned()]);
    assert_eq!((loaded.config, left_alone), (Config::default(), expected));
}

#[test]
fn an_unreadable_file_is_a_read_error_not_a_missing_one() {
    use std::os::unix::fs::PermissionsExt;
    if !permissions_apply() {
        return;
    }
    let dir = TempDir::new();
    let path = dir.path().join("config.toml");
    fs::write(&path, "version = 1\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

    let loaded = Config::load(&path);

    let read_error = matches!(
        loaded.error.as_ref().map(ConfigError::problem),
        Some(Problem::Read(err)) if err.kind() == std::io::ErrorKind::PermissionDenied
    );
    assert_eq!((loaded.config, read_error), (Config::default(), true));
}

/// Loads `path` on another thread, failing if that takes longer than a few
/// seconds: a load that blocks must fail the test, not hang it.
fn load_within_bound(path: PathBuf) -> Loaded {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // The receiver is gone only after a timeout, which fails the test.
        let _ = tx.send(Config::load(&path));
    });
    rx.recv_timeout(Duration::from_secs(5))
        .expect("loading the config blocked")
}

/// The reason a load failed without a line.
fn reason_of(loaded: &Loaded) -> Option<&str> {
    match loaded.error.as_ref().map(ConfigError::problem) {
        Some(Problem::Invalid { line: None, reason }) => Some(reason),
        _ => None,
    }
}

#[test]
fn only_a_regular_file_is_read() {
    let dir = TempDir::new();
    let fifo = dir.path().join("fifo.toml");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());

    for path in [fifo, dir.path().to_owned(), PathBuf::from("/dev/zero")] {
        let loaded = load_within_bound(path.clone());
        let reason = reason_of(&loaded);
        assert_eq!(
            reason,
            Some("the path is not a regular file"),
            "{}",
            path.display()
        );
    }
}

#[test]
fn a_file_over_64_kib_is_rejected_unread() {
    let dir = TempDir::new();
    let path = dir.path().join("config.toml");
    let at_limit = format!("version = 1\n{}", "#".repeat(64 * 1024 - 12));
    fs::write(&path, &at_limit).unwrap();
    assert!(
        Config::load(&path).error.is_none(),
        "exactly 64 KiB is read"
    );

    fs::write(&path, format!("{at_limit}#")).unwrap();
    let loaded = Config::load(&path);
    assert_eq!(reason_of(&loaded), Some("the file is larger than 64 KiB"));
}

#[test]
fn a_file_that_is_not_text_is_invalid() {
    let dir = TempDir::new();
    let path = dir.path().join("config.toml");
    fs::write(&path, b"version = 1\n\xff\xfe").unwrap();

    let error = Config::load(&path).error.unwrap();

    assert!(matches!(
        error.problem(),
        Problem::Invalid { line: None, .. }
    ));
}

// The path

#[test]
fn the_path_follows_xdg_config_home_then_home() {
    let xdg = Some("/xdg/auto-clicker/config.toml");
    let fallback = Some("/home/me/.config/auto-clicker/config.toml");
    let cases = [
        (Some("/xdg"), Some("/home/me"), xdg),
        (None, Some("/home/me"), fallback),
        (Some(""), Some("/home/me"), fallback),
        (Some("relative"), Some("/home/me"), fallback),
        (None, Some("relative"), None),
        (None, None, None),
    ];
    for (xdg_config_home, home, expected) in cases {
        let path = config_path_from(xdg_config_home.map(Into::into), home.map(Into::into));
        assert_eq!(
            path,
            expected.map(PathBuf::from),
            "{xdg_config_home:?} {home:?}"
        );
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
