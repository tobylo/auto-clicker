use auto_clicker_core::config::PositionDraft;
use auto_clicker_core::config::RouteName;
use auto_clicker_core::config::{Config, Refusal};
use auto_clicker_core::controller::{Hotkey, Routes};
use auto_clicker_core::hotkeys::Binding;
use auto_clicker_core::recording::{Layout, Output};
use auto_clicker_core::window::{Answer, ShownHotkeys, Since, Spot};

use super::*;

fn view(settings: Draft, answer: Option<Answer>) -> WindowView {
    let config = Config::default();
    WindowView {
        running: settings.input,
        routes: Routes::default(),
        settings,
        activity: Activity::Idle,
        hotkeys: ShownHotkeys {
            toggle_clicking: Hotkey::Bound(config.hotkeys.toggle_clicking),
            toggle_recording: Hotkey::Bound(config.hotkeys.toggle_recording),
            toggle_playback: Hotkey::Bound(config.hotkeys.toggle_playback),
        },
        no_recording: None,
        problems: Vec::new(),
        recordings: Default::default(),
        screens: None,
        since: Since::default(),
        answer,
    }
}

fn defaults() -> Draft {
    Draft::from(&Config::default())
}

fn fields() -> Fields {
    Fields::from_draft(&defaults(), '.')
}

/// A state that took the default settings.
fn started() -> State {
    let mut state = State::default();
    let shown = state.view(&view(defaults(), None));
    assert_eq!(shown.fields, Some(defaults()));
    state
}

fn answered(state: &mut State, id: u64, outcome: Outcome) -> Shown {
    state.view(&view(defaults(), Some(Answer { id, outcome })))
}

fn saved(state: &mut State) -> Request {
    state.save(&fields()).unwrap().unwrap()
}

/// Checks that `state` sends nothing the user asks: no save, capture or
/// request for the desktop's settings.
fn asks_nothing(state: &mut State) {
    assert!(!state.may_ask());
    let asked = (
        state.save(&fields()),
        state.capture(Target::Hotkey(0)),
        state.configure(),
        state.recording(file(), "label".to_owned(), RecordingAction::Play),
        state.locate(spot(1.0, 1.0), panel_rect(), Since::default()),
    );
    assert_eq!(asked, (None, None, None, None, None));
}

fn file() -> FileName {
    FileName::parse("2026-11-09_18-04-05.123.json").unwrap()
}

/// What a request asks of a recording.
fn recording_of(request: Request) -> (FileName, RecordingAction) {
    match request.ask {
        Ask::Recording { file, action } => (file, action),
        other => panic!("{other:?}"),
    }
}

/// The save a request asks for: its settings, and what it agreed to.
fn save_of(request: Request) -> (Draft, bool, Vec<Action>) {
    match request.ask {
        Ask::Save {
            settings,
            replace,
            without_modifier,
        } => (settings, replace, without_modifier),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_fields_fill_when_the_saved_settings_change_and_only_then() {
    let mut state = started();
    assert_eq!(state.view(&view(defaults(), None)), Shown::default());
    let mut changed = defaults();
    changed.tray_start_delay = 9;
    let shown = state.view(&view(changed.clone(), None));
    assert_eq!(shown.fields, Some(changed));
}

#[test]
fn nothing_is_asked_before_the_settings_came() {
    asks_nothing(&mut State::default());
}

#[test]
fn a_save_waits_for_its_own_answer_and_nothing_else_is_asked_meanwhile() {
    let mut state = started();
    let request = saved(&mut state);
    assert!(state.waiting());
    asks_nothing(&mut state);
    // An answer to some other request is not this one's.
    assert_eq!(
        answered(&mut state, request.id + 1, Outcome::Saved).note,
        None
    );
    assert!(state.waiting());
    assert_eq!(
        answered(&mut state, request.id, Outcome::Saved).note,
        Some(Note::Saved)
    );
    assert!(!state.waiting() && state.may_ask());
}

#[test]
fn answers_become_notes() {
    let refusal = Refusal::new(Field::Interval, Reason::Speed);
    let cases = [
        (
            Outcome::SavedUnsynced(Quoted::new("disk")),
            Note::SavedUnsynced(Quoted::new("disk")),
        ),
        (Outcome::Busy, Note::Busy),
        (
            Outcome::Failed(Failure::BadToken),
            Note::Failed(Failure::BadToken),
        ),
        (
            Outcome::Refused(refusal),
            Note::Refused(Field::Interval, Reason::Speed),
        ),
        (
            Outcome::Unavailable(Unavailability::NoShortcuts),
            Note::Unavailable(Unavailability::NoShortcuts),
        ),
        (Outcome::Configured, Note::Configured),
        (
            Outcome::CaptureEnded(CaptureEnd::TimedOut),
            Note::CaptureEnded(CaptureEnd::TimedOut),
        ),
        (
            Outcome::Recording(RecordingOutcome::Renamed),
            Note::Recording(RecordingOutcome::Renamed),
        ),
        (
            Outcome::Recording(RecordingOutcome::NotPlaying(None)),
            Note::Recording(RecordingOutcome::NotPlaying(None)),
        ),
    ];
    for (outcome, note) in cases {
        let mut state = started();
        let request = state.configure().unwrap();
        assert_eq!(
            answered(&mut state, request.id, outcome).note,
            Some(note.clone())
        );
        let error = matches!(
            note,
            Note::Failed(_)
                | Note::Refused(..)
                | Note::Unavailable(_)
                | Note::Recording(
                    RecordingOutcome::NotChanged(..) | RecordingOutcome::NotPlaying(_)
                )
        );
        assert_eq!(note.is_error(), error, "{note:?}");
    }
    assert!(Note::Unreadable(Unreadable::Key).is_error());
}

#[test]
fn a_question_blocks_other_requests_and_a_yes_saves_the_fields_as_they_are_now() {
    let mut state = started();
    let first = saved(&mut state);
    let ask = Confirm::Replace(Stale::Changed);
    assert_eq!(
        answered(&mut state, first.id, Outcome::Confirm(ask)).note,
        None
    );
    assert_eq!(state.question(), Some(&Question::Replace(Stale::Changed)));
    asks_nothing(&mut state);
    let mut now = fields();
    now.tray_delay = 7;
    let again = state.agree(&now).unwrap().unwrap();
    let again_id = again.id;
    let (settings, replace, _) = save_of(again);
    assert!(replace && settings.tray_start_delay == 7);
    assert_eq!(state.question(), None);
    // The second question comes on top of the first agreement.
    let plain = Confirm::WithoutModifier(vec![Action::TogglePlayback]);
    answered(&mut state, again_id, Outcome::Confirm(plain));
    let (_, replace, without_modifier) = save_of(state.agree(&now).unwrap().unwrap());
    assert_eq!(
        (replace, without_modifier),
        (true, vec![Action::TogglePlayback])
    );
}

#[test]
fn a_yes_while_an_answer_is_awaited_sends_nothing() {
    let mut state = started();
    let first = saved(&mut state);
    answered(
        &mut state,
        first.id,
        Outcome::Confirm(Confirm::Replace(Stale::Changed)),
    );
    let again = state.agree(&fields()).unwrap().unwrap();
    answered(
        &mut state,
        again.id,
        Outcome::Confirm(Confirm::Replace(Stale::Changed)),
    );
    let pending = state.agree(&fields()).unwrap().unwrap();
    assert!(state.waiting());
    assert_eq!(state.agree(&fields()), None);
    assert_eq!(
        answered(&mut state, pending.id, Outcome::Saved).note,
        Some(Note::Saved)
    );
}

#[test]
fn a_no_sends_nothing() {
    let mut state = started();
    let first = saved(&mut state);
    answered(
        &mut state,
        first.id,
        Outcome::Confirm(Confirm::Replace(Stale::Unusable)),
    );
    state.decline();
    assert_eq!(state.question(), None);
    assert_eq!(state.agree(&fields()), None);
    assert!(state.may_ask());
}

#[test]
fn a_captured_hotkey_is_kept_for_the_next_save_and_a_key_is_noted() {
    let mut state = started();
    let capture = state.capture(Target::Hotkey(1)).unwrap();
    assert_eq!(capture.ask, Ask::Capture(Capture::Hotkey));
    let ctrl_f9 = Binding::parse("Ctrl+F9").unwrap();
    let note = answered(&mut state, capture.id, Outcome::CapturedHotkey(ctrl_f9)).note;
    assert_eq!(note, Some(Note::CapturedHotkey("Ctrl+F9".to_owned())));
    let key = state.capture(Target::Key).unwrap();
    assert_eq!(key.ask, Ask::Capture(Capture::Key));
    let note = answered(&mut state, key.id, Outcome::CapturedKey(30)).note;
    assert_eq!(note, Some(Note::CapturedKey(30)));
    assert_eq!(state.hotkeys().unwrap().toggle_recording, "Ctrl+F9");
    let save = saved(&mut state);
    let Ask::Save { settings, .. } = save.ask else {
        panic!("{save:?}");
    };
    assert_eq!(settings.hotkeys.toggle_recording, "Ctrl+F9");
}

#[test]
fn the_view_says_whether_idle_where_hotkeys_come_from_and_what_waits_for_a_restart() {
    // `auto` in the config, and the desktop's shortcuts picked at start.
    let mut shown = view(defaults(), None);
    assert_eq!(shown.running.hotkeys, RouteName::Auto);
    shown.routes.hotkeys = Route::Portal;
    assert!(idle(&shown) && desktop_hotkeys(&shown) && !routes_pending(&shown));
    // The keyboards picked, and the config changed since the start.
    shown.routes.hotkeys = Route::Kernel;
    shown.settings.input.hotkeys = RouteName::Portal;
    shown.activity = Activity::Clicking;
    assert!(!idle(&shown) && !desktop_hotkeys(&shown) && routes_pending(&shown));
}

#[test]
fn a_save_coming_back_fills_the_fields_as_the_daemon_saved_them() {
    let mut state = started();
    let mut edited = fields();
    edited.interval_unit = 1;
    edited.interval_text = "20".to_owned();
    let request = state.save(&edited).unwrap().unwrap();
    // The daemon keeps whole milliseconds as such.
    let mut saved = defaults();
    saved.clicker.interval = auto_clicker_core::config::IntervalDraft::Ms(50);
    let answer = Answer {
        id: request.id,
        outcome: Outcome::Saved,
    };
    let back = state.view(&view(saved.clone(), Some(answer)));
    assert_eq!(
        back,
        Shown {
            fields: Some(saved),
            note: Some(Note::Saved)
        }
    );
}

#[test]
fn a_yes_with_a_field_that_cannot_be_read_keeps_the_question() {
    let mut state = started();
    let first = saved(&mut state);
    answered(
        &mut state,
        first.id,
        Outcome::Confirm(Confirm::Replace(Stale::Changed)),
    );
    let mut wrong = fields();
    wrong.speed_text = "fast".to_owned();
    assert_eq!(state.agree(&wrong), Some(Err(Unreadable::Speed)));
    assert_eq!(state.question(), Some(&Question::Replace(Stale::Changed)));
    let agreed = state.agree(&fields());
    assert!(matches!(agreed, Some(Ok(_))) && state.question().is_none());
}

#[test]
fn a_rename_a_choice_or_a_play_goes_at_once_and_waits_for_its_answer() {
    let actions = [
        RecordingAction::Rename {
            name: "Farm loop".to_owned(),
        },
        RecordingAction::Choose,
        RecordingAction::Play,
    ];
    for action in actions {
        let mut state = started();
        let request = state
            .recording(file(), "label".to_owned(), action.clone())
            .unwrap();
        let id = request.id;
        assert_eq!(recording_of(request), (file(), action));
        asks_nothing(&mut state);
        let shown = answered(&mut state, id, Outcome::Recording(RecordingOutcome::Chosen));
        assert_eq!(shown.note, Some(Note::Recording(RecordingOutcome::Chosen)));
        assert!(state.may_ask());
    }
}

#[test]
fn a_delete_asks_first_and_goes_only_on_a_yes() {
    let mut state = started();
    let label = "Farm loop (2026-11-09 18:04:05)".to_owned();
    let asked = state.recording(file(), label.clone(), RecordingAction::Delete);
    assert_eq!(asked, None);
    assert_eq!(
        state.question(),
        Some(&Question::Delete(file(), label.clone()))
    );
    asks_nothing(&mut state);

    let request = state.agree(&fields()).unwrap().unwrap();
    assert_eq!(recording_of(request), (file(), RecordingAction::Delete));
    assert_eq!(state.question(), None);
    assert!(state.waiting());

    let mut state = started();
    state.recording(file(), label, RecordingAction::Delete);
    state.decline();
    assert_eq!(state.question(), None);
    assert_eq!(state.agree(&fields()), None);
    assert!(state.may_ask());
}

#[test]
fn selecting_another_recording_calls_off_a_delete_asked_about() {
    let mut state = started();
    let other = FileName::parse("2026-11-10_07-00-00.000.json").unwrap();
    state.recording(file(), "label".to_owned(), RecordingAction::Delete);
    // The same one again keeps the question.
    state.selected(Some(&file()));
    assert!(matches!(state.question(), Some(Question::Delete(..))));

    state.selected(Some(&other));

    assert_eq!(state.question(), None);
    assert_eq!(state.agree(&fields()), None);
    // A save's question stays whatever is selected.
    let save = saved(&mut state);
    answered(
        &mut state,
        save.id,
        Outcome::Confirm(Confirm::Replace(Stale::Changed)),
    );
    state.selected(Some(&other));
    state.selected(None);
    assert_eq!(state.question(), Some(&Question::Replace(Stale::Changed)));
}

#[test]
fn a_selected_recording_leaving_the_list_calls_off_its_delete() {
    let mut state = started();
    state.recording(file(), "label".to_owned(), RecordingAction::Delete);

    state.selected(None);

    assert_eq!(state.question(), None);
    assert_eq!(state.agree(&fields()), None);
}

fn spot(x: f64, y: f64) -> Spot {
    Spot::new(x, y).unwrap()
}

fn panel_rect() -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: 2048,
        height: 1280,
    }
}

fn panel() -> Layout {
    Layout::new(vec![Output::new(panel_rect(), 1250).unwrap()]).unwrap()
}

#[test]
fn a_located_pick_is_noted_and_kept_for_the_next_save_with_its_screens() {
    let mut state = started();
    let since = Since {
        runs: 2,
        layouts: 5,
    };
    let request = state
        .locate(spot(1024.5, 640.25), panel_rect(), since)
        .unwrap();
    assert_eq!(
        request.ask,
        Ask::Locate {
            at: spot(1024.5, 640.25),
            on: panel_rect(),
            since
        }
    );
    asks_nothing(&mut state);
    let located = Outcome::Located(Located {
        x: 32768,
        y: 32768,
        screens: panel(),
    });
    let shown = answered(&mut state, request.id, located);
    assert_eq!(shown.note, Some(Note::Picked { x: 32768, y: 32768 }));
    assert_eq!(shown.fields, None);
    let mut filled = fields();
    filled.position_fixed = true;
    filled.position_x = 32768;
    filled.position_y = 32768;
    let (settings, _, _) = save_of(state.save(&filled).unwrap().unwrap());
    assert_eq!(
        settings.clicker.position,
        PositionDraft::Fixed {
            x: 32768,
            y: 32768,
            screens: Some(panel())
        }
    );
}

#[test]
fn a_refused_pick_is_noted_and_keeps_nothing() {
    let mut state = started();
    let request = state
        .locate(spot(1.0, 1.0), panel_rect(), Since::default())
        .unwrap();
    let refused = Outcome::Unavailable(Unavailability::ScreensChanged);
    let shown = answered(&mut state, request.id, refused);
    assert_eq!(
        shown.note,
        Some(Note::Unavailable(Unavailability::ScreensChanged))
    );
    let mut typed = fields();
    typed.position_fixed = true;
    let (settings, _, _) = save_of(state.save(&typed).unwrap().unwrap());
    assert_eq!(
        settings.clicker.position,
        PositionDraft::Fixed {
            x: 0,
            y: 0,
            screens: None
        }
    );
}

#[test]
fn nothing_else_is_asked_while_a_pick_is_up() {
    let mut state = started();
    state.set_picking(true);
    asks_nothing(&mut state);
    state.set_picking(false);
    assert!(state.may_ask());
    assert!(
        state
            .locate(spot(1.0, 1.0), panel_rect(), Since::default())
            .is_some()
    );
}
