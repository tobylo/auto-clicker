//! Version 3: positions Hyprland reported (W01 brief, "Settled by W01d").

use super::positions::{at, two_screens};
use super::*;

fn from_hyprland() -> Recording {
    let events = vec![
        event(0, at(640, 400)),
        event(1_200, Change::Move { dx: 3, dy: -2 }),
        event(1_900, at(643, 398)),
    ];
    Recording::with_positions(
        "n".to_owned(),
        created(),
        5_000,
        Track {
            layout: two_screens(),
            source: PointerSource::Hyprland,
            events,
        },
    )
    .unwrap()
}

const HEADER: &str = r#""name":"n","created":"2026-11-09T18:04:05.123+01:00","duration_us":5000"#;
const LAYOUT: &str = r#""layout":[{"x":0,"y":0,"width":2048,"height":1280,"scale_milli":1250}]"#;

/// A file of `version` with `fields` between the header and the events.
fn file(version: u64, fields: &str) -> String {
    format!(r#"{{"version":{version},{HEADER}{fields},"events":[[0,"at",1,1]]}}"#)
}

#[test]
fn hyprlands_positions_are_written_as_version_3_and_round_trip() {
    let recording = from_hyprland();
    let text = json(&recording);
    let start = r#"{"version":3,"name":"n","created":"2026-11-09T18:04:05.123+01:00","duration_us":5000,"positions_from":"hyprland","layout":["#;
    assert!(text.starts_with(start), "{text}");
    let read = Recording::from_json(text.as_bytes()).unwrap();
    assert_eq!(read, recording);
    assert_eq!(read.positions_from(), Some(PointerSource::Hyprland));
}

#[test]
fn version_2_is_kwins_and_a_recording_without_positions_has_no_source() {
    let read = Recording::from_json(file(2, &format!(",{LAYOUT}")).as_bytes()).unwrap();
    assert_eq!(read.positions_from(), Some(PointerSource::Kwin));
    assert_eq!(sample().positions_from(), None);
}

#[test]
fn a_source_in_any_other_form_is_refused() {
    let hyprland = r#","positions_from":"hyprland""#;
    let cases = [
        (
            file(3, &format!(",{LAYOUT}")),
            "a version 3 recording's positions are from \"hyprland\"",
        ),
        (
            file(3, &format!(r#","positions_from":"kwin",{LAYOUT}"#)),
            "a version 3 recording's positions are from \"hyprland\"",
        ),
        (
            file(3, &format!(r#","positions_from":"Hyprland",{LAYOUT}"#)),
            "a version 3 recording's positions are from \"hyprland\"",
        ),
        (file(3, hyprland), "a version 3 recording needs a layout"),
        (
            file(2, &format!("{hyprland},{LAYOUT}")),
            "only a version 3 recording says where its positions came from",
        ),
        (
            file(1, hyprland),
            "only a version 3 recording says where its positions came from",
        ),
    ];
    for (text, why) in cases {
        assert_eq!(refused(&text), why, "{text}");
    }
    let null = file(3, &format!(r#","positions_from":null,{LAYOUT}"#));
    assert!(refused(&null).starts_with("the file does not have a recording's layout"));
}

#[test]
fn floored_positions_play_from_the_middle_of_their_pixel() {
    let layout = two_screens();
    let point = Point { x: 2047, y: 0 };
    let middle = layout.absolute_at(2047.5, 0.5, layout.outputs()[0].rect());
    assert_eq!(PointerSource::Hyprland.absolute(&layout, point), middle);
    assert_eq!(
        PointerSource::Kwin.absolute(&layout, point),
        layout.absolute(point)
    );
    assert_ne!(middle, layout.absolute(point));
}
