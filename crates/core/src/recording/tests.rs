//! The recording type and its file format: round trips, and every way a
//! file is refused.

use super::*;
use crate::input::{Button, Key};

pub(crate) fn created() -> Created {
    Created::parse("2026-11-09T18:04:05.123+01:00").unwrap()
}

pub(crate) fn key(code: u16) -> Held {
    Held::Key(Key::new(code).unwrap())
}

const W: u16 = 17;
const LEFT: Held = Held::Button(Button::Left);

fn event(at: u64, change: Change) -> Event {
    Event { at, change }
}

/// One of every kind of event.
pub(crate) fn sample() -> Recording {
    let events = vec![
        event(0, Change::Press(key(W))),
        event(1_200, Change::Move { dx: 3, dy: -2 }),
        event(1_200, Change::Press(LEFT)),
        event(480_000, Change::Wheel(-1)),
        event(500_000, Change::Release(LEFT)),
        event(2_500_000, Change::Release(key(W))),
    ];
    Recording::new(
        "Recording 2026-11-09 18:04".to_owned(),
        created(),
        3_000_000,
        events,
    )
    .unwrap()
}

fn json(recording: &Recording) -> String {
    let mut out = Vec::new();
    recording.write_json(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn the_file_has_a_header_and_one_compact_event_per_line() {
    let expected = r#"{"version":1,"name":"Recording 2026-11-09 18:04","created":"2026-11-09T18:04:05.123+01:00","duration_us":3000000,"events":[
[0,"press","KEY_W"],
[1200,"move",3,-2],
[1200,"press","BTN_LEFT"],
[480000,"wheel",-1],
[500000,"release","BTN_LEFT"],
[2500000,"release","KEY_W"]
]}
"#;
    assert_eq!(json(&sample()), expected);
}

#[test]
fn every_kind_of_event_round_trips() {
    let recording = sample();
    let read = Recording::from_json(json(&recording).as_bytes()).unwrap();
    assert_eq!(read, recording);
}

#[test]
fn an_empty_recording_and_an_escaped_name_round_trip() {
    let recording = Recording::new(
        "A \"quoted\" \\ name ✓".to_owned(),
        created(),
        0,
        Vec::new(),
    )
    .unwrap();
    let text = json(&recording);
    assert!(text.ends_with("\"events\":[\n]}\n"), "{text}");
    assert_eq!(Recording::from_json(text.as_bytes()).unwrap(), recording);
}

#[test]
fn aliases_are_read_and_canonical_names_written() {
    let text = sample_text().replace("BTN_LEFT", "BTN_MOUSE");
    let read = Recording::from_json(text.as_bytes()).unwrap();
    assert_eq!(read, sample());
    assert!(json(&read).contains("BTN_LEFT"));
}

fn sample_text() -> String {
    json(&sample())
}

/// The reason `text` is refused.
fn refused(text: &str) -> String {
    Recording::from_json(text.as_bytes())
        .unwrap_err()
        .to_string()
}

fn with_events(events: &str) -> String {
    format!(
        r#"{{"version":1,"name":"n","created":"2026-11-09T18:04:05.123+01:00","duration_us":5000,"events":[{events}]}}"#
    )
}

#[test]
fn unknown_versions_are_refused_whatever_else_the_file_holds() {
    for (text, version) in [
        (r#"{"version":3,"anything":[1,2]}"#, 3),
        (&sample_text().replace("\"version\":1", "\"version\":0"), 0),
    ] {
        assert_eq!(
            refused(text),
            format!(
                "the file is a version {version} recording; this program reads versions 1 and 2"
            )
        );
    }
}

#[test]
fn malformed_events_are_refused_with_their_place() {
    let cases = [
        (
            r#"[0,"press","KEY_W"],[10,"move",1,1],[5,"release","KEY_W"]"#,
            "events[2] is earlier than the event before it",
        ),
        (
            r#"[0,"release","KEY_W"]"#,
            "events[0] releases a key or button that is not down",
        ),
        (
            r#"[0,"press","KEY_W"],[1,"press","KEY_W"],[2,"release","KEY_W"]"#,
            "events[1] presses a key or button that is already down",
        ),
        (
            r#"[0,"press","KEY_W"]"#,
            "the recording ends with a key or button still down",
        ),
        (
            r#"[0,"press","KEY_NOPE"]"#,
            "events[0] names an unknown kind, key or button",
        ),
        (
            r#"[0,"press","key_w"]"#,
            "events[0] names an unknown kind, key or button",
        ),
        (
            r#"[0,"press","BTN_FORWARD"]"#,
            "events[0] names an unknown kind, key or button",
        ),
        (
            r#"[0,"jump",1]"#,
            "events[0] names an unknown kind, key or button",
        ),
        (r#"[0,"move",0,0]"#, "events[0] moves by nothing"),
        (r#"[0,"wheel",0]"#, "events[0] turns the wheel by nothing"),
        (
            r#"[6000,"wheel",1]"#,
            "events[0] is later than the end of the recording",
        ),
    ];
    for (events, reason) in cases {
        assert_eq!(refused(&with_events(events)), reason, "{events}");
    }
}

#[test]
fn a_wrong_layout_is_refused_without_quoting_the_file() {
    let layout = "the file does not have a recording's layout";
    let cases = [
        (with_events(r#"[0,"move",1]"#), layout),
        (with_events(r#"[0,"move",1,2,3]"#), layout),
        (with_events(r#"[0,"wheel",4294967296]"#), layout),
        (with_events(r#"[-1,"wheel",1]"#), layout),
        (with_events(r#"[0,"press",30]"#), layout),
        (with_events(r#"["KEY_SECRET","press","KEY_W"]"#), layout),
        (with_events("{}"), layout),
        (sample_text().replace("\"name\"", "\"title\""), layout),
        (
            sample_text().replace("{\"version\":1,", "{\"version\":1,\"extra\":true,"),
            layout,
        ),
        (
            sample_text().replace("\"events\":[", "\"events\":[ KEY_SECRET"),
            "the file is not valid JSON",
        ),
        (sample_text()[..40].to_owned(), "the file ends too early"),
    ];
    for (text, reason) in cases {
        let message = refused(&text);
        assert!(message.starts_with(reason), "{message} for {text}");
        assert!(message.contains("line "), "{message}");
        assert!(!message.contains("KEY_"), "{message}");
    }
}

#[test]
fn the_header_is_checked() {
    let cases = [
        (
            sample_text().replace("2026-11-09T18:04:05.123+01:00", "yesterday"),
            "the recording's creation time is malformed",
        ),
        (
            sample_text().replace("\"Recording 2026-11-09 18:04\"", "\" \""),
            "the recording has no name",
        ),
        (
            sample_text().replace("Recording 2026-11-09 18:04", "two\\nlines"),
            "the recording's name holds a control character",
        ),
        (
            sample_text().replace("Recording 2026-11-09 18:04", &"x".repeat(201)),
            "the recording's name is longer than 200 characters",
        ),
        (
            sample_text().replace("\"duration_us\":3000000", "\"duration_us\":3600000001"),
            "the recording is longer than 60 minutes",
        ),
        (
            sample_text().replace("\"duration_us\":3000000", "\"duration_us\":2000000"),
            "events[5] is later than the end of the recording",
        ),
    ];
    for (text, reason) in cases {
        assert_eq!(refused(&text), reason);
    }
    let longest = sample_text().replace("\"duration_us\":3000000", "\"duration_us\":3600000000");
    assert!(Recording::from_json(longest.as_bytes()).is_ok());
    let longest_name = sample_text().replace("Recording 2026-11-09 18:04", &"é".repeat(200));
    assert!(Recording::from_json(longest_name.as_bytes()).is_ok());
}

#[test]
fn a_recording_named_with_an_unnamed_code_is_refused() {
    let unnamed = vec![
        event(0, Change::Press(key(84))),
        event(1, Change::Release(key(84))),
    ];
    let err = Recording::new("n".to_owned(), created(), 1, unnamed).unwrap_err();
    assert_eq!(err.to_string(), "events[0] names no key or button");
}

#[test]
fn debug_output_leaves_the_events_out() {
    let shown = format!("{:?}", sample());
    assert!(shown.contains("events: 6"), "{shown}");
    assert!(!shown.contains("Key") && !shown.contains("17"), "{shown}");
}

pub(crate) mod positions;
