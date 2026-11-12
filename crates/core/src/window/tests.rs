use super::*;
use crate::config::Reason;
use crate::config::{Config, Field};
use crate::controller::Blocked;
use crate::controller::{Route, Routes, WINDOW_RECORDINGS};
use crate::hotkeys::{Binding, Unavailable};
use crate::message::{ChangeFailure, ConfigProblem, RecordingChange};

fn file(text: &str) -> FileName {
    FileName::parse(text).unwrap()
}

/// A listed recording named `name`, made at 18:04:05 on day `day`.
fn listed(day: usize, name: Option<&str>) -> Listed {
    let file = file(&format!("2026-11-{:02}_18-04-05.123.json", day % 28 + 1));
    Listed {
        label: file.titled(name),
        name: name.map(str::to_owned),
        file,
    }
}

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
        recordings: RecordingsView {
            listed: vec![listed(1, Some("Farm loop")), listed(0, None)],
            more: 3,
            current: Some(file("2026-11-02_18-04-05.123.json")),
        },
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
        Outcome::Recording(RecordingOutcome::Renamed),
        Outcome::Recording(RecordingOutcome::Deleted),
        Outcome::Recording(RecordingOutcome::Chosen),
        Outcome::Recording(RecordingOutcome::Playing),
        Outcome::Recording(RecordingOutcome::NotChanged(
            RecordingChange::Rename,
            ChangeFailure::Missing,
        )),
        Outcome::Recording(RecordingOutcome::Unsynced(
            RecordingChange::Delete,
            Quoted::new("no flush"),
        )),
        Outcome::Recording(RecordingOutcome::NotPlaying(Some(Notice::NothingToPlay))),
        Outcome::Recording(RecordingOutcome::NotPlaying(None)),
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
        Ask::Recording {
            file: file("2026-11-09_18-04-05.123-1.json"),
            action: RecordingAction::Rename {
                name: "Farm \"loop\"".to_owned(),
            },
        },
        Ask::Recording {
            file: file("2026-11-09_18-04-05.123.json"),
            action: RecordingAction::Delete,
        },
        Ask::Recording {
            file: file("2026-11-09_18-04-05.123.json"),
            action: RecordingAction::Choose,
        },
        Ask::Recording {
            file: file("2026-11-09_18-04-05.123.json"),
            action: RecordingAction::Play,
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
fn a_recording_is_named_only_by_its_file_names_one_form() {
    let ask = Ask::Recording {
        file: file("2026-11-09_18-04-05.123.json"),
        action: RecordingAction::Delete,
    };
    let mut value = serde_json::to_value(Request { id: 1, ask }).unwrap();
    for named in [
        "../2026-11-09_18-04-05.123.json",
        "2026-11-09_18-04-05.123.json/",
        "config.toml",
        "",
    ] {
        value["ask"]["recording"]["file"] = serde_json::json!(named);
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(parse::<Request>(&bytes).is_err(), "{named}");
    }
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
    // The longest names a recording may have, in the widest characters
    // JSON writes as they are: four bytes each.
    let longest = "😀".repeat(200);
    widest.recordings = RecordingsView {
        listed: (0..WINDOW_RECORDINGS)
            .map(|day| listed(day, Some(&longest)))
            .collect(),
        more: u64::MAX,
        current: Some(file("2026-11-09_18-04-05.123-9.json")),
    };
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
        Outcome::Recording(RecordingOutcome::NotChanged(
            RecordingChange::Delete,
            ChangeFailure::Folder(Quoted::new(worst.clone())),
        )),
        Outcome::Recording(RecordingOutcome::Unsynced(
            RecordingChange::Rename,
            Quoted::new(worst.clone()),
        )),
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
    let cut = format!("{}…", "a".repeat(MAX_TEXT));
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
    let cut = format!("{}…", "a".repeat(MAX_TEXT));
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

#[test]
fn what_comes_of_a_recording_reads_as_the_window_says_it() {
    let cases = [
        (RecordingOutcome::Renamed, "Renamed."),
        (RecordingOutcome::Deleted, "Deleted."),
        (
            RecordingOutcome::Chosen,
            "The playback hotkey plays this recording now.",
        ),
        (RecordingOutcome::Playing, "Playing."),
        (
            RecordingOutcome::Unsynced(RecordingChange::Rename, Quoted::new("EIO")),
            "Renamed, but it may not survive a power loss: EIO",
        ),
        (
            RecordingOutcome::NotChanged(RecordingChange::Delete, ChangeFailure::Missing),
            "Not deleted: the file is no longer there",
        ),
        (
            RecordingOutcome::NotChanged(RecordingChange::Rename, ChangeFailure::Missing),
            "Not renamed: the file is no longer there",
        ),
        (
            RecordingOutcome::NotPlaying(Some(Notice::NothingToPlay)),
            "Nothing to play: no recordings yet",
        ),
        (RecordingOutcome::NotPlaying(None), "Not playing"),
    ];
    for (outcome, english) in cases {
        assert_eq!(outcome.to_string(), english);
    }
}

#[test]
fn a_notice_reads_back_as_sent_with_its_key_and_refuses_a_key_that_is_none() {
    use crate::hotkeys::Refusal;
    use crate::input::Key;
    use crate::message::{Hindrance, Start};
    let notice = Notice::Refused {
        start: Start::Clicking,
        why: Hindrance::OwnKey(Refusal::Modifier(Key::new(42).unwrap())),
    };
    let outcome = Outcome::Recording(RecordingOutcome::NotPlaying(Some(notice)));
    let answer = Answer { id: 2, outcome };
    let sent = body(&answer);
    assert!(String::from_utf8_lossy(&sent).contains("\"modifier\":42"));
    assert_eq!(parse::<Answer>(&sent).unwrap(), answer);
    let none = String::from_utf8_lossy(&sent).replace("\"modifier\":42", "\"modifier\":0");
    assert!(parse::<Answer>(none.as_bytes()).is_err());
}
