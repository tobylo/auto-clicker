use super::*;
use crate::config::{Config, Field};
use crate::hotkeys::{Binding, Unavailable};

fn save(id: u64) -> Request {
    Request {
        id,
        ask: Ask::Save {
            settings: Draft::from(&Config::default()),
            replace: false,
            without_modifier: true,
        },
    }
}

fn view() -> WindowView {
    let config = Config::default();
    WindowView {
        settings: Draft::from(&config),
        running: Draft::from(&config).input,
        activity: Activity::Clicking,
        hotkeys: ShownHotkeys {
            toggle_clicking: Hotkey::Bound(Binding::parse("Ctrl+F6").unwrap()),
            toggle_recording: Hotkey::Desktop(None),
            toggle_playback: Hotkey::Desktop(Some("Meta+P".to_owned())),
        },
        no_recording: Some(NoRecording::Keyboards(Unavailable::NoPermission)),
        problems: vec!["a problem".to_owned()],
        answer: Some(Answer {
            id: 7,
            outcome: Outcome::Refused(Refusal::new(
                Field::Hotkey(Action::TogglePlayback),
                "no".to_owned(),
            )),
        }),
    }
}

/// The line without its newline.
fn body(message: &impl Serialize) -> Vec<u8> {
    let mut bytes = line(message).unwrap();
    assert_eq!(bytes.pop(), Some(b'\n'));
    assert!(!bytes.contains(&b'\n'));
    bytes
}

#[test]
fn requests_and_views_read_back_as_sent() {
    assert_eq!(parse::<Request>(&body(&save(3))).unwrap(), save(3));
    assert_eq!(parse::<WindowView>(&body(&view())).unwrap(), view());
}

#[test]
fn every_answer_reads_back_as_sent() {
    let outcomes = [
        Outcome::Saved,
        Outcome::Failed("disk full".to_owned()),
        Outcome::Confirm(Confirm::Replace(Stale::Changed)),
        Outcome::Confirm(Confirm::Replace(Stale::Unusable)),
        Outcome::Confirm(Confirm::WithoutModifier(Action::ToggleRecording)),
    ];
    for outcome in outcomes {
        let answer = Answer { id: 1, outcome };
        assert_eq!(parse::<Answer>(&body(&answer)).unwrap(), answer);
    }
}

#[test]
fn a_request_with_an_unknown_field_is_not_read() {
    let mut value = serde_json::to_value(save(1)).unwrap();
    value["extra"] = serde_json::json!(true);
    assert!(parse::<Request>(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn a_shown_binding_is_read_only_in_its_one_form() {
    let mut value = serde_json::to_value(view()).unwrap();
    value["hotkeys"]["toggle_clicking"] = serde_json::json!({ "bound": "Ctrl+Nope" });
    assert!(parse::<WindowView>(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn lines_longer_than_the_limit_are_neither_read_nor_sent() {
    let mut long = view();
    long.problems = vec!["x".repeat(MAX_LINE)];
    assert!(line(&long).is_err());
    let padded = format!("{{\"id\":1,\"ask\":\"{}\"}}", "x".repeat(MAX_LINE));
    let err = parse::<Request>(padded.as_bytes()).unwrap_err();
    assert!(err.to_string().contains("longer than"), "{err}");
}
