//! Version 2: positions and their layout, in memory and in the file.

use super::*;

/// An output at `[x, y, width, height]`, at `scale_milli`.
pub(crate) fn screen([x, y, width, height]: [u32; 4], scale_milli: u32) -> Output {
    let rect = Rect {
        x,
        y,
        width,
        height,
    };
    Output::new(rect, scale_milli).unwrap()
}

/// The laptop panel at scale 1.25 with DP-1 to its right.
pub(crate) fn two_screens() -> Layout {
    Layout::new(vec![
        screen([0, 0, 2048, 1280], 1250),
        screen([2048, 0, 3440, 1440], 1000),
    ])
    .unwrap()
}

pub(crate) fn at(x: u32, y: u32) -> Change {
    Change::Position(Point { x, y })
}

fn positioned() -> Recording {
    let events = vec![
        event(0, at(640, 400)),
        event(1_200, Change::Move { dx: 3, dy: -2 }),
        event(1_900, at(643, 398)),
        event(52_000, Change::Press(LEFT)),
        event(52_000, Change::Release(LEFT)),
    ];
    Recording::with_positions(
        "Recording 2026-11-09 18:04".to_owned(),
        created(),
        52_000,
        Track {
            layout: two_screens(),
            events,
        },
    )
    .unwrap()
}

const HEADER_2: &str =
    r#"{"version":2,"name":"n","created":"2026-11-09T18:04:05.123+01:00","duration_us":5000"#;
const PANEL: &str = r#"{"x":0,"y":0,"width":2048,"height":1280,"scale_milli":1250}"#;

/// A version 2 file with `layout` (the field's value, or nothing) and
/// `events`.
fn version_2(layout: Option<&str>, events: &str) -> String {
    let layout = layout.map_or(String::new(), |layout| format!(r#","layout":{layout}"#));
    format!(r#"{HEADER_2}{layout},"events":[{events}]}}"#)
}

#[test]
fn a_recording_with_positions_is_written_as_version_2() {
    let expected = r#"{"version":2,"name":"Recording 2026-11-09 18:04","created":"2026-11-09T18:04:05.123+01:00","duration_us":52000,"layout":[{"x":0,"y":0,"width":2048,"height":1280,"scale_milli":1250},{"x":2048,"y":0,"width":3440,"height":1440,"scale_milli":1000}],"events":[
[0,"at",640,400],
[1200,"move",3,-2],
[1900,"at",643,398],
[52000,"press","BTN_LEFT"],
[52000,"release","BTN_LEFT"]
]}
"#;
    assert_eq!(json(&positioned()), expected);
}

#[test]
fn version_2_round_trips() {
    let recording = positioned();
    let read = Recording::from_json(json(&recording).as_bytes()).unwrap();
    assert_eq!(read, recording);
    assert_eq!(read.layout(), Some(&two_screens()));
}

#[test]
fn a_recording_without_positions_is_still_version_1() {
    assert!(sample_text().starts_with(r#"{"version":1,"#));
    assert_eq!(sample().layout(), None);
}

#[test]
fn a_layout_without_positions_or_a_position_without_a_layout_is_refused() {
    let moves = vec![event(0, Change::Move { dx: 1, dy: 0 })];
    let err = Recording::with_positions(
        "n".into(),
        created(),
        0,
        Track {
            layout: two_screens(),
            events: moves,
        },
    );
    assert_eq!(
        err.unwrap_err().to_string(),
        "the recording has a layout but no positions"
    );
    let err = Recording::new("n".into(), created(), 0, vec![event(0, at(1, 1))]);
    assert_eq!(
        err.unwrap_err().to_string(),
        "events[0] is a position on no screen of the recording's layout"
    );
}

#[test]
fn version_1_refuses_a_layout_and_positions() {
    let with_layout = sample_text().replacen(
        r#""duration_us":3000000"#,
        &format!(r#""duration_us":3000000,"layout":[{PANEL}]"#),
        1,
    );
    assert_eq!(refused(&with_layout), "a version 1 recording has no layout");
    assert_eq!(
        refused(&with_events(r#"[0,"move",1,0],[10,"at",1,1]"#)),
        "events[1] is a position, which a version 1 recording cannot hold"
    );
}

#[test]
fn version_2_refuses_what_is_not_a_layout_or_a_position_on_it() {
    let at = r#"[0,"at",10,10]"#;
    let screen = |x, y, width, height| {
        format!(r#"{{"x":{x},"y":{y},"width":{width},"height":{height},"scale_milli":1000}}"#)
    };
    let sixteen = vec![screen(0, 0, 10, 10); MAX_OUTPUTS];
    let seventeen = vec![screen(0, 0, 10, 10); MAX_OUTPUTS + 1];
    let cases = [
        (version_2(None, at), "a version 2 recording needs a layout"),
        (
            version_2(Some(&format!("[{PANEL}]")), r#"[0,"move",1,0]"#),
            "the recording has a layout but no positions",
        ),
        (version_2(Some("[]"), at), "the layout has no screens"),
        (
            version_2(Some(&format!("[{}]", screen(0, 0, 0, 10))), at),
            "a screen of the layout has no area",
        ),
        (
            version_2(Some(&format!("[{}]", screen(0, 0, 10, 0))), at),
            "a screen of the layout has no area",
        ),
        (
            version_2(
                Some(r#"[{"x":0,"y":0,"width":10,"height":10,"scale_milli":0}]"#),
                at,
            ),
            "a screen of the layout has no scale",
        ),
        (
            version_2(Some(&format!("[{}]", seventeen.join(","))), at),
            "the layout has more than 16 screens",
        ),
        (
            version_2(
                Some(&format!("[{}]", screen(1, 0, 100, 100))),
                r#"[0,"at",50,50]"#,
            ),
            "the layout does not start at the top-left corner",
        ),
        (
            version_2(
                Some(&format!("[{}]", screen(0, 0, 10, 10))),
                r#"[0,"at",10,5]"#,
            ),
            "events[0] is a position on no screen of the recording's layout",
        ),
        (
            version_2(Some(&format!("[{}]", screen(4294967295_u32, 0, 1, 1))), at),
            "a screen of the layout reaches too far",
        ),
    ];
    for (text, reason) in cases {
        assert_eq!(refused(&text), reason, "{text}");
    }
    let most = version_2(Some(&format!("[{}]", sixteen.join(","))), r#"[0,"at",9,9]"#);
    assert_eq!(
        Recording::from_json(most.as_bytes())
            .unwrap()
            .layout()
            .unwrap()
            .outputs()
            .len(),
        16
    );
}

#[test]
fn a_layout_or_position_of_the_wrong_shape_is_refused_as_malformed() {
    let cases = [
        version_2(
            Some(r#"[{"x":0,"y":0,"width":10,"height":10,"scale_milli":1000,"name":"DP-1"}]"#),
            r#"[0,"at",1,1]"#,
        ),
        version_2(
            Some(r#"[{"x":0,"y":0,"width":10,"height":10}]"#),
            r#"[0,"at",1,1]"#,
        ),
        version_2(
            Some(r#"[{"x":-1,"y":0,"width":10,"height":10,"scale_milli":1}]"#),
            r#"[0,"at",1,1]"#,
        ),
        version_2(Some(&format!("[{PANEL}]")), r#"[0,"at",1]"#),
        version_2(Some(&format!("[{PANEL}]")), r#"[0,"at",1,1,1]"#),
        version_2(Some(&format!("[{PANEL}]")), r#"[0,"at",-1,1]"#),
        version_2(Some(&format!("[{PANEL}]")), r#"[0,"at",1.5,1]"#),
    ];
    for text in cases {
        let reason = refused(&text);
        assert!(
            reason.starts_with("the file does not have a recording's layout"),
            "{text}: {reason}"
        );
    }
}

#[test]
fn an_event_counts_its_line_at_the_latest_time_with_its_separator() {
    let line = r#"[3600000000,"at",65535,65535]"#;
    assert_eq!(event_bytes(at(65535, 65535)), line.len() as u64 + 2);
    let line = r#"[3600000000,"release","BTN_LEFT"]"#;
    assert_eq!(event_bytes(Change::Release(LEFT)), line.len() as u64 + 2);
}

#[test]
fn the_events_of_a_file_take_no_more_than_they_count() {
    let recording = positioned();
    let text = json(&recording);
    let (_, body) = text.split_once("\"events\":[").unwrap();
    let counted: u64 = recording
        .events()
        .iter()
        .map(|e| event_bytes(e.change))
        .sum();
    assert!(counted >= body.len() as u64, "{counted} < {}", body.len());
}
