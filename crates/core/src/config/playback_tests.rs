//! `[playback]` in the config file: the speed and how often a recording
//! plays; and `[on_screen]`, what the on-screen display shows.

use std::num::NonZeroU32;

use super::*;
use crate::clicker::Repeat;
use crate::playback::{PlaybackSettings, Speed};

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

/// The `[playback]` section `section` gives.
fn playback(section: &str) -> PlaybackSettings {
    parse(&format!("version = 1\n[playback]\n{section}\n")).playback
}

#[test]
fn playback_defaults_to_once_at_the_recorded_speed() {
    assert_eq!(parse("version = 1\n").playback, PlaybackSettings::default());
    assert_eq!(playback(""), PlaybackSettings::default());
}

#[test]
fn playback_speed_is_a_number_from_a_quarter_to_four() {
    let speed = |ratio| Speed::from_ratio(ratio).unwrap();
    for (section, expected) in [
        ("speed = 0.25", speed(0.25)),
        ("speed = 4.0", speed(4.0)),
        // A whole number needs no decimal point.
        ("speed = 2", speed(2.0)),
        ("speed = 1.5", speed(1.5)),
    ] {
        assert_eq!(playback(section).speed, expected, "{section}");
    }
    for section in [
        "speed = 0.2",
        "speed = 4.5",
        "speed = 0",
        "speed = -1.0",
        "speed = nan",
        "speed = inf",
    ] {
        let (line, reason) = invalid(&format!("version = 1\n[playback]\n{section}\n"));
        assert_eq!(
            (line, reason.as_str()),
            (Some(3), "speed must be from 0.25 to 4"),
            "{section}"
        );
    }
    let (_, reason) = invalid("version = 1\n[playback]\nspeed = \"fast\"\n");
    assert!(reason.contains("expected f64"), "{reason}");
}

#[test]
fn playback_repeats_a_number_of_plays_or_until_stopped() {
    let times = |count| Repeat::Times(NonZeroU32::new(count).unwrap());
    assert_eq!(playback("repeat = 5").repeat, times(5));
    assert_eq!(playback("repeat = 4294967295").repeat, times(u32::MAX));
    assert_eq!(
        playback("repeat = \"until_stopped\"").repeat,
        Repeat::UntilStopped
    );
    for section in ["repeat = 0", "repeat = -3", "repeat = 4294967296"] {
        let (line, reason) = invalid(&format!("version = 1\n[playback]\n{section}\n"));
        assert_eq!(line, Some(3), "{section}");
        assert!(reason.contains("\"until_stopped\""), "{section}: {reason}");
    }
    let (_, reason) = invalid("version = 1\n[playback]\nrepeat = \"twice\"\n");
    assert!(reason.contains("a number of plays from 1"), "{reason}");
}

/// The `[on_screen]` section `section` gives.
fn on_screen(section: &str) -> OnScreenSettings {
    parse(&format!("version = 1\n[on_screen]\n{section}\n")).on_screen
}

#[test]
fn everything_shows_on_the_screen_in_the_top_left_unless_set_otherwise() {
    let all = OnScreenSettings {
        countdown: true,
        clicking: true,
        recording: true,
        playback: true,
        corner: Corner::TopLeft,
    };
    assert_eq!(parse("version = 1\n").on_screen, all);
    assert_eq!(on_screen(""), all);
    assert!(on_screen("countdown = true").countdown);
    assert!(!on_screen("countdown = false").countdown);
    // Each switch turns its own part off, and nothing else.
    let off = |key: &str| on_screen(&format!("{key} = false"));
    assert_eq!(
        off("clicking"),
        OnScreenSettings {
            clicking: false,
            ..all
        }
    );
    assert_eq!(
        off("recording"),
        OnScreenSettings {
            recording: false,
            ..all
        }
    );
    assert_eq!(
        off("playback"),
        OnScreenSettings {
            playback: false,
            ..all
        }
    );
    assert!(on_screen("clicking = true").clicking);
}

#[test]
fn each_corner_is_read_by_its_name() {
    for corner in Corner::ALL {
        let section = format!("corner = \"{}\"", corner.word());
        assert_eq!(on_screen(&section).corner, corner, "{section}");
    }
}

#[test]
fn an_on_screen_value_that_is_not_true_or_false_is_refused_at_its_line() {
    for (section, says) in [
        ("countdown = \"no\"", "boolean"),
        ("countdown = 0", "boolean"),
        ("clicking = \"yes\"", "boolean"),
        ("recording = 1", "boolean"),
        ("playback = \"off\"", "boolean"),
        ("badge = true", "unknown field"),
    ] {
        let (line, reason) = invalid(&format!("version = 1\n[on_screen]\n{section}\n"));
        assert_eq!(line, Some(3), "{section}");
        assert!(reason.contains(says), "{section}: {reason}");
    }
}

#[test]
fn an_unknown_corner_is_refused_at_its_line_with_the_names() {
    let names = "`top_left`, `top_right`, `bottom_left`, `bottom_right`";
    for (section, says) in [
        ("corner = \"middle\"", names),
        ("corner = \"Top_Left\"", names),
        ("corner = \"top-left\"", names),
        ("corner = \"\"", names),
        // Not a word at all, as for any other word in the file.
        ("corner = 1", "string"),
    ] {
        let (line, reason) = invalid(&format!("version = 1\n[on_screen]\n{section}\n"));
        assert_eq!(line, Some(3), "{section}");
        assert!(reason.contains(says), "{section}: {reason}");
    }
}
