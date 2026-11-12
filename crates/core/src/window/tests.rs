use super::*;
use crate::config::Reason;
use crate::config::{Config, Field};
use crate::controller::Blocked;
use crate::controller::{Route, Routes};
use crate::hotkeys::{Binding, Unavailable};
use crate::message::ConfigProblem;

fn save(id: u64) -> Request {
    Request {
        id,
        ask: Ask::Save {
            settings: Draft::from(&Config::default()),
            replace: false,
            without_modifier: vec![Action::ToggleClicking],
        },
    }
}

fn view() -> WindowView {
    let config = Config::default();
    WindowView {
        settings: Draft::from(&config),
        running: Draft::from(&config).input,
        routes: Routes {
            output: Route::Kernel,
            hotkeys: Route::Portal,
        },
        activity: Activity::Clicking,
        hotkeys: ShownHotkeys {
            toggle_clicking: Hotkey::Bound(Binding::parse("Ctrl+F6").unwrap()),
            toggle_recording: Hotkey::Desktop(None),
            toggle_playback: Hotkey::Desktop(Some("Meta+P".to_owned())),
        },
        no_recording: Some(NoRecording::Keyboards(Unavailable::NoPermission)),
        problems: vec![Problem::Unlisted(Quoted::new("a problem"))],
        answer: Some(Answer {
            id: 7,
            outcome: Outcome::Refused(Refusal::new(
                Field::Hotkey(Action::TogglePlayback),
                Reason::Binding(Quoted::new("no")),
            )),
        }),
    }
}

/// The view showing `trigger` as the desktop's playback hotkey: text only
/// [`WindowView::bounded`] cuts.
fn with_trigger(trigger: String) -> WindowView {
    let mut shown = view();
    shown.hotkeys.toggle_playback = Hotkey::Desktop(Some(trigger));
    shown
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
        Outcome::SavedUnsynced(Quoted::new("no fsync")),
        Outcome::Failed(Failure::Write(Quoted::new("disk full"))),
        Outcome::Refused(Refusal::new(Field::Interval, Reason::Speed)),
        Outcome::Confirm(Confirm::Replace(Stale::Changed)),
        Outcome::Confirm(Confirm::Replace(Stale::Unusable)),
        Outcome::Confirm(Confirm::WithoutModifier(vec![Action::ToggleRecording])),
        Outcome::CapturedHotkey(Binding::parse("Ctrl+F9").unwrap()),
        Outcome::CapturedKey(57),
        Outcome::CaptureEnded(CaptureEnd::TimedOut),
        Outcome::Configured,
        Outcome::Unavailable(Unavailability::Hotkeys(Unavailable::NoKeyboard)),
    ];
    for outcome in outcomes {
        let answer = Answer { id: 1, outcome };
        assert_eq!(parse::<Answer>(&body(&answer)).unwrap(), answer);
    }
}

#[test]
fn every_request_reads_back_as_sent() {
    let asks = [
        Ask::Capture(Capture::Hotkey),
        Ask::Capture(Capture::Key),
        Ask::ConfigureShortcuts {
            activation_token: Some("token".to_owned()),
        },
        Ask::ConfigureShortcuts {
            activation_token: None,
        },
    ];
    for ask in asks {
        let request = Request { id: 4, ask };
        assert_eq!(parse::<Request>(&body(&request)).unwrap(), request);
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
    let long = with_trigger("x".repeat(MAX_LINE));
    assert!(line(&long).is_err());
    let padded = format!("{{\"id\":1,\"ask\":\"{}\"}}", "x".repeat(MAX_LINE));
    let err = parse::<Request>(padded.as_bytes()).unwrap_err();
    assert!(err.to_string().contains("longer than"), "{err}");
}

/// A view whose line, newline included, is `size` bytes long.
fn view_of_size(size: usize) -> WindowView {
    let bare = line(&with_trigger(String::new())).unwrap().len();
    with_trigger("x".repeat(size - bare))
}

#[test]
fn a_line_of_exactly_the_limit_is_sent_and_read_and_one_more_byte_is_not() {
    let largest = view_of_size(MAX_LINE);
    let sent = line(&largest).unwrap();
    assert_eq!(sent.len(), MAX_LINE);
    assert_eq!(parse::<WindowView>(&sent[..MAX_LINE - 1]).unwrap(), largest);
    let over = view_of_size(MAX_LINE + 1);
    assert!(line(&over).is_err());
    // Valid JSON, refused for its length alone.
    let body = serde_json::to_vec(&over).unwrap();
    assert_eq!(body.len(), MAX_LINE);
    assert!(parse::<WindowView>(&body).is_err());
}

#[test]
fn long_messages_are_cut_so_every_view_fits_a_line() {
    // Control characters are the longest to escape: six bytes each.
    let worst = "\u{1}".repeat(MAX_LINE);
    let desktop = || Hotkey::Desktop(Some(worst.clone()));
    let mut widest = view();
    widest.hotkeys = ShownHotkeys {
        toggle_clicking: desktop(),
        toggle_recording: desktop(),
        toggle_playback: desktop(),
    };
    // The problem that quotes the most: two texts.
    let config = Problem::Config(ConfigProblem::Invalid {
        file: Quoted::new(worst.clone()),
        line: Some(usize::MAX),
        why: Quoted::new(worst.clone()),
    });
    widest.problems = vec![config; MAX_PROBLEMS + 1];
    widest.answer = Some(Answer {
        id: u64::MAX,
        outcome: Outcome::Refused(Refusal::new(
            Field::Key,
            Reason::Binding(Quoted::new(worst.clone())),
        )),
    });
    for why in [
        Unavailable::ReadFailed(worst.clone().into()),
        Unavailable::WatchFailed(worst.clone().into()),
        Unavailable::Desktop(worst.clone().into()),
    ] {
        widest.no_recording = Some(NoRecording::Keyboards(why));
        let shown = widest.clone().bounded();
        assert_eq!(shown.problems.len(), MAX_PROBLEMS);
        // As the daemon sends it: inside an update.
        assert!(line(&Update::View(Box::new(shown))).is_ok());
    }
    assert_eq!(bounded(&worst).chars().count(), MAX_TEXT + 1);
    assert_eq!(bounded("short"), "short");
    for outcome in [
        Outcome::Failed(Failure::Desktop(Quoted::new(worst.clone()))),
        Outcome::Failed(Failure::Hotkeys(Unavailable::Desktop(worst.clone().into()))),
        Outcome::SavedUnsynced(Quoted::new(worst.clone())),
        Outcome::Unavailable(Unavailability::Hotkeys(Unavailable::ReadFailed(
            worst.clone().into(),
        ))),
        Outcome::Unavailable(Unavailability::Blocked(Blocked::Unknown(
            worst.clone().into(),
        ))),
    ] {
        let answer = Answer { id: 1, outcome };
        assert!(line(&answer).unwrap().len() < MAX_TEXT * 7);
    }
}

#[test]
fn every_problem_cut_for_the_window_fits_and_reads_back_as_values() {
    let worst = "\u{1}".repeat(MAX_LINE);
    let problems = [
        Problem::Blocked(Blocked::Unknown(worst.clone().into())),
        Problem::Hotkeys(Unavailable::ReadFailed(worst.clone().into())),
        Problem::Hotkeys(Unavailable::WatchFailed(worst.clone().into())),
        Problem::Hotkeys(Unavailable::Desktop(worst.clone().into())),
        Problem::Unlisted(Quoted::new(worst.clone())),
    ];
    for problem in problems {
        let mut shown = view();
        shown.problems = vec![problem];
        let sent = line(&shown.bounded()).unwrap();
        assert!(sent.len() < MAX_TEXT * 7 + 2048, "{}", sent.len());
        let read = parse::<WindowView>(&sent[..sent.len() - 1]).unwrap();
        // The quoted text, cut, inside the problem's own words.
        let shown = read.problems[0].to_string();
        assert!(shown.chars().count() < MAX_TEXT + 100, "{shown}");
        assert!(shown.contains(&bounded(&worst)), "{shown}");
    }
}

#[test]
fn a_message_is_read_only_as_one_of_its_values_and_no_longer_than_cut() {
    let answer = |outcome: serde_json::Value| {
        let value = serde_json::json!({ "id": 1, "outcome": outcome });
        parse::<Answer>(&serde_json::to_vec(&value).unwrap())
    };
    let cut = "a".repeat(MAX_TEXT + 1);
    let over = "a".repeat(MAX_TEXT + 2);
    assert!(answer(serde_json::json!({ "failed": { "write": cut } })).is_ok());
    assert!(answer(serde_json::json!({ "failed": { "write": over } })).is_err());
    assert!(answer(serde_json::json!({ "failed": "disk full" })).is_err());
    assert!(answer(serde_json::json!({ "failed": { "nope": "x" } })).is_err());
    let refused = |reason: serde_json::Value| {
        answer(serde_json::json!({ "refused": { "field": "key", "reason": reason } }))
    };
    assert!(refused(serde_json::json!({ "key_code": { "code": 300 } })).is_ok());
    assert!(refused(serde_json::json!({ "binding": over })).is_err());
    assert!(refused(serde_json::json!("key code 300 is not from 1 to 248")).is_err());
    assert!(refused(serde_json::json!({ "key_code": { "code": "300" } })).is_err());
}

#[test]
fn a_message_with_an_unknown_field_is_not_read() {
    let mut value = serde_json::to_value(view()).unwrap();
    value["problems"] = serde_json::json!([
        { "config": { "invalid": { "file": "c", "line": 1, "why": "w", "extra": 1 } } }
    ]);
    assert!(parse::<WindowView>(&serde_json::to_vec(&value).unwrap()).is_err());
    value["problems"] = serde_json::json!([
        { "config": { "invalid": { "file": "c", "line": 1, "why": "w" } } }
    ]);
    assert!(parse::<WindowView>(&serde_json::to_vec(&value).unwrap()).is_ok());
}

#[test]
fn a_nested_reason_longer_than_cut_is_not_read() {
    let with = |problem: serde_json::Value| {
        let mut value = serde_json::to_value(view()).unwrap();
        value["problems"] = serde_json::json!([problem]);
        parse::<WindowView>(&serde_json::to_vec(&value).unwrap())
    };
    let cut = "a".repeat(MAX_TEXT + 1);
    let over = "a".repeat(MAX_TEXT + 2);
    for text in [&cut, &over] {
        let fits = text == &cut;
        let problems = [
            serde_json::json!({ "blocked": { "unknown": text } }),
            serde_json::json!({ "hotkeys": { "read_failed": text } }),
            serde_json::json!({ "hotkeys": { "watch_failed": text } }),
            serde_json::json!({ "hotkeys": { "desktop": text } }),
            serde_json::json!({ "input": { "portal": text } }),
        ];
        for problem in problems {
            assert_eq!(with(problem.clone()).is_ok(), fits, "{problem}");
        }
    }
}
