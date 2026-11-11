use super::*;
use crate::config::{Config, Field};
use crate::controller::{Route, Routes};
use crate::hotkeys::{Binding, Unavailable};

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
        Outcome::Confirm(Confirm::WithoutModifier(vec![Action::ToggleRecording])),
        Outcome::CapturedHotkey(Binding::parse("Ctrl+F9").unwrap()),
        Outcome::CapturedKey(57),
        Outcome::CaptureEnded(CaptureEnd::TimedOut),
        Outcome::Configured,
        Outcome::Unavailable("no keyboard".to_owned()),
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
    let mut long = view();
    long.problems = vec!["x".repeat(MAX_LINE)];
    assert!(line(&long).is_err());
    let padded = format!("{{\"id\":1,\"ask\":\"{}\"}}", "x".repeat(MAX_LINE));
    let err = parse::<Request>(padded.as_bytes()).unwrap_err();
    assert!(err.to_string().contains("longer than"), "{err}");
}

/// A view whose line, newline included, is `size` bytes long.
fn view_of_size(size: usize) -> WindowView {
    let mut sized = view();
    sized.problems = vec![String::new()];
    let bare = line(&sized).unwrap().len();
    sized.problems = vec!["x".repeat(size - bare)];
    sized
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
    widest.problems = vec![worst.clone(); MAX_PROBLEMS + 1];
    widest.answer = Some(Answer {
        id: u64::MAX,
        outcome: Outcome::Refused(Refusal::new(Field::Key, worst.clone())),
    });
    for why in [
        Unavailable::ReadFailed(worst.clone()),
        Unavailable::WatchFailed(worst.clone()),
        Unavailable::Desktop(worst.clone()),
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
        Outcome::Failed(worst.clone()),
        Outcome::SavedUnsynced(worst.clone()),
    ] {
        let answer = Answer {
            id: 1,
            outcome: outcome.bounded(),
        };
        assert!(line(&answer).unwrap().len() < MAX_TEXT * 7);
    }
}
