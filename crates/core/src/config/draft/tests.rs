use std::num::NonZeroU32;
use std::time::Duration;

use super::*;
use crate::clicker::Jitter;
use crate::config::{ButtonName, HotkeyRoute, OutputRoute, render};
use crate::input::{Button, Key};
use crate::playback::Speed;

fn key(name: &str) -> Key {
    Key::from_name(name).unwrap()
}

fn binding(text: &str) -> Binding {
    Binding::parse(text).unwrap()
}

fn times(n: u32) -> Repeat {
    Repeat::Times(NonZeroU32::new(n).unwrap())
}

fn with_clicker(change: impl FnOnce(&mut ClickerSettings)) -> Config {
    let mut config = Config::default();
    change(&mut config.clicker);
    config
}

fn per_second(rate: f64, mode: Mode) -> Interval {
    check::clicks_per_second(rate, mode).unwrap()
}

/// A case's name and its config.
type Named = (&'static str, Config);

/// Every option the file has, each away from its default at least once.
fn every_option() -> Vec<Named> {
    let mut configs = vec![("defaults", Config::default())];
    configs.extend(modes_and_buttons());
    configs.extend(clicker_values());
    configs.extend(other_sections());
    configs.extend(playback_values());
    configs
}

fn modes_and_buttons() -> Vec<Named> {
    let modes = [
        ("mode click", Mode::Click),
        ("mode double_click", Mode::DoubleClick),
        ("mode hold_button", Mode::HoldButton),
        ("mode key_repeat", Mode::KeyRepeat),
        ("mode hold_key", Mode::HoldKey),
    ];
    let buttons = [
        ("button right", Button::Right),
        ("button middle", Button::Middle),
        ("button back", Button::Side),
        ("button forward", Button::Extra),
    ];
    let modes = modes.map(|(name, mode)| (name, with_clicker(|c| c.mode = mode)));
    let buttons = buttons.map(|(name, button)| (name, with_clicker(|c| c.button = button)));
    modes.into_iter().chain(buttons).collect()
}

fn clicker_values() -> Vec<Named> {
    let interval = |d| Interval::new(d).unwrap();
    vec![
        (
            "interval 10 ms",
            with_clicker(|c| c.interval = interval(Duration::from_millis(10))),
        ),
        (
            "interval one day",
            with_clicker(|c| c.interval = interval(Duration::from_secs(86_400))),
        ),
        (
            "interval 3 per second",
            with_clicker(|c| c.interval = per_second(3.0, Mode::Click)),
        ),
        (
            "interval 7.3 per second",
            with_clicker(|c| c.interval = per_second(7.3, Mode::Click)),
        ),
        (
            "jitter 50",
            with_clicker(|c| c.jitter = Jitter::new(50).unwrap()),
        ),
        ("key named", with_clicker(|c| c.key = key("f13"))),
        (
            "key by code",
            with_clicker(|c| c.key = Key::new(113).unwrap()),
        ),
        (
            "position fixed",
            with_clicker(|c| c.position = Position::Fixed(AbsolutePosition { x: 0, y: 65535 })),
        ),
        (
            "repeat counted",
            with_clicker(|c| c.repeat = times(u32::MAX)),
        ),
    ]
}

fn other_sections() -> Vec<Named> {
    let hotkeys = Bindings {
        toggle_clicking: binding("Ctrl+Alt+Meta+Shift+102nd"),
        toggle_recording: binding("F13"),
        toggle_playback: binding("Alt+Space"),
    };
    let routes = |output, hotkeys| InputSettings { output, hotkeys };
    let d = Config::default;
    vec![
        (
            "tray delay 0",
            Config {
                tray_start_delay: Duration::ZERO,
                ..d()
            },
        ),
        (
            "tray delay max",
            Config {
                tray_start_delay: Duration::from_secs(3600),
                ..d()
            },
        ),
        ("hotkeys", Config { hotkeys, ..d() }),
        (
            "routes portal portal",
            Config {
                input: routes(OutputRoute::Portal, HotkeyRoute::Portal),
                ..d()
            },
        ),
        (
            "routes kernel portal",
            Config {
                input: routes(OutputRoute::Kernel, HotkeyRoute::Portal),
                ..d()
            },
        ),
        (
            "routes portal kernel",
            Config {
                input: routes(OutputRoute::Portal, HotkeyRoute::Kernel),
                ..d()
            },
        ),
    ]
}

fn playback_values() -> Vec<Named> {
    let playback = |ratio, repeat| PlaybackSettings {
        speed: Speed::from_ratio(ratio).unwrap(),
        repeat,
    };
    let d = Config::default;
    vec![
        (
            "playback slowest",
            Config {
                playback: playback(0.25, Repeat::UntilStopped),
                ..d()
            },
        ),
        (
            "playback 1.333",
            Config {
                playback: playback(1.333, times(3)),
                ..d()
            },
        ),
        (
            "playback fastest",
            Config {
                playback: playback(4.0, times(1)),
                ..d()
            },
        ),
    ]
}

#[test]
fn every_option_reads_back_from_the_written_file() {
    for (name, config) in every_option() {
        let text = render(&config);
        assert_eq!(Config::parse(&text).ok(), Some(config), "{name}:\n{text}");
    }
}

#[test]
fn every_option_survives_the_draft_and_the_wire() {
    for (name, config) in every_option() {
        let draft = Draft::from(&config);
        let wire = serde_json::to_string(&draft).unwrap();
        let back: Draft = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, draft, "{name}");
        assert_eq!(back.to_config(), Ok(config), "{name}");
    }
}

#[test]
fn rates_read_back_to_the_same_interval() {
    let mut rng = fastrand::Rng::with_seed(17);
    let fixed = [
        100.0,
        99.999,
        3.0,
        1.0 / 3.0,
        0.001,
        0.0000116,
        33.3,
        47.9,
        1.0 / 86_400.0,
    ];
    let random = (0..2000).map(|_| 0.00002 + rng.f64() * 100.0);
    for rate in fixed.into_iter().chain(random) {
        let config = with_clicker(|c| c.interval = per_second(rate, Mode::Click));
        let text = render(&config);
        assert_eq!(
            Config::parse(&text).ok(),
            Some(config),
            "rate {rate}:\n{text}"
        );
    }
}

#[test]
fn whole_milliseconds_are_written_as_milliseconds() {
    let config = with_clicker(|c| c.interval = per_second(4.0, Mode::Click));
    assert_eq!(
        spell_interval(config.clicker.interval),
        IntervalDraft::Ms(250)
    );
    assert!(render(&config).contains("\ninterval_ms = 250\n"));
}

#[test]
fn the_file_starts_with_a_header_and_writes_speed_with_a_point() {
    let text = render(&Config::default());
    assert!(text.starts_with("# auto-clicker settings, written by its Settings window.\n"));
    assert!(text.contains("\nspeed = 1.0\n"), "{text}");
}

#[test]
fn written_words_need_no_escaping() {
    let plain = |word: &str| {
        word.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'_')
    };
    let names = (1..=248).filter_map(|code| Key::new(code).and_then(Key::name));
    for name in names.chain(crate::hotkeys::Modifier::ALL.map(|m| m.name())) {
        assert!(plain(name), "{name}");
    }
}

#[test]
fn names_spell_as_the_file_does() {
    for mode in [
        ModeName::Click,
        ModeName::DoubleClick,
        ModeName::HoldButton,
        ModeName::KeyRepeat,
        ModeName::HoldKey,
    ] {
        assert_eq!(
            serde_json::to_string(&mode).unwrap(),
            format!("\"{}\"", mode.word())
        );
    }
    for button in [
        ButtonName::Left,
        ButtonName::Right,
        ButtonName::Middle,
        ButtonName::Back,
        ButtonName::Forward,
    ] {
        assert_eq!(
            serde_json::to_string(&button).unwrap(),
            format!("\"{}\"", button.word())
        );
    }
    for route in [RouteName::Kernel, RouteName::Portal] {
        assert_eq!(
            serde_json::to_string(&route).unwrap(),
            format!("\"{}\"", route.word())
        );
    }
}

/// A case's name, how it changes the defaults, and the field refused.
type Case = (&'static str, fn(&mut Draft), Field);

fn refused(change: impl FnOnce(&mut Draft)) -> Field {
    let mut draft = Draft::from(&Config::default());
    change(&mut draft);
    draft.to_config().unwrap_err().field
}

#[test]
fn each_invalid_value_is_refused_at_its_field() {
    let cases: [Case; 13] = [
        (
            "delay over an hour",
            |d| d.tray_start_delay = 3601,
            Field::TrayStartDelay,
        ),
        (
            "interval below the floor",
            |d| d.clicker.interval = IntervalDraft::Ms(9),
            Field::Interval,
        ),
        (
            "interval over a day",
            |d| d.clicker.interval = IntervalDraft::Ms(86_400_001),
            Field::Interval,
        ),
        (
            "double click below its floor",
            |d| {
                d.clicker.mode = ModeName::DoubleClick;
                d.clicker.interval = IntervalDraft::Ms(19);
            },
            Field::Interval,
        ),
        (
            "rate zero",
            |d| d.clicker.interval = IntervalDraft::PerSecond(0.0),
            Field::Interval,
        ),
        (
            "jitter over 50",
            |d| d.clicker.jitter_percent = 51,
            Field::Jitter,
        ),
        ("key code 0", |d| d.clicker.key = 0, Field::Key),
        ("key code 249", |d| d.clicker.key = 249, Field::Key),
        (
            "repeat 0",
            |d| d.clicker.repeat = CountDraft::Times(0),
            Field::Repeat,
        ),
        (
            "repeat too many",
            |d| d.clicker.repeat = CountDraft::Times(u64::from(u32::MAX) + 1),
            Field::Repeat,
        ),
        (
            "unknown hotkey",
            |d| d.hotkeys.toggle_recording = "Ctrl+Nope".to_owned(),
            Field::Hotkey(Action::ToggleRecording),
        ),
        ("speed too fast", |d| d.playback.speed = 4.01, Field::Speed),
        (
            "playback repeat 0",
            |d| d.playback.repeat = CountDraft::Times(0),
            Field::PlaybackRepeat,
        ),
    ];
    for (name, change, field) in cases {
        assert_eq!(refused(change), field, "{name}");
    }
}

#[test]
fn two_actions_on_one_hotkey_are_refused_at_the_later() {
    let field = refused(|d| d.hotkeys.toggle_playback = "ctrl+shift+f6".to_owned());
    assert_eq!(field, Field::Hotkey(Action::TogglePlayback));
}

#[test]
fn a_hotkey_on_the_clicker_key_is_refused_except_on_the_portal_route() {
    let on_space = |d: &mut Draft| {
        d.clicker.mode = ModeName::KeyRepeat;
        d.hotkeys.toggle_recording = "Ctrl+Space".to_owned();
    };
    for route in [RouteName::Auto, RouteName::Kernel] {
        let field = refused(|d| {
            on_space(d);
            d.input.hotkeys = route;
        });
        assert_eq!(field, Field::Hotkey(Action::ToggleRecording), "{route:?}");
    }
    let mut draft = Draft::from(&Config::default());
    on_space(&mut draft);
    draft.input.hotkeys = RouteName::Portal;
    assert!(draft.to_config().is_ok());
}

#[test]
fn a_draft_with_an_unknown_field_is_not_read() {
    let mut wire = serde_json::to_value(Draft::from(&Config::default())).unwrap();
    wire["clicker"]["extra"] = serde_json::json!(1);
    assert!(serde_json::from_value::<Draft>(wire).is_err());
}
