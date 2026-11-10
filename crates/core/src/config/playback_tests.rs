//! `[playback]` in the config file: the speed and how often a recording
//! plays.

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
