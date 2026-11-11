use auto_clicker_core::config::{Config, Refusal};
use auto_clicker_core::controller::Hotkey;
use auto_clicker_core::hotkeys::Binding;
use auto_clicker_core::window::{Answer, ShownHotkeys};

use super::*;

fn view(settings: Draft, answer: Option<Answer>) -> WindowView {
    let config = Config::default();
    WindowView {
        running: settings.input,
        settings,
        activity: Activity::Idle,
        hotkeys: ShownHotkeys {
            toggle_clicking: Hotkey::Bound(config.hotkeys.toggle_clicking),
            toggle_recording: Hotkey::Bound(config.hotkeys.toggle_recording),
            toggle_playback: Hotkey::Bound(config.hotkeys.toggle_playback),
        },
        no_recording: None,
        problems: Vec::new(),
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
    let mut state = State::default();
    assert!(!state.may_ask());
    assert_eq!(state.save(&fields()), None);
    assert_eq!(state.capture(Target::Key), None);
    assert_eq!(state.configure(), None);
}

#[test]
fn a_save_waits_for_its_own_answer_and_nothing_else_is_asked_meanwhile() {
    let mut state = started();
    let request = saved(&mut state);
    assert!(state.waiting() && !state.may_ask());
    assert_eq!(state.save(&fields()), None);
    assert_eq!(state.capture(Target::Hotkey(0)), None);
    assert_eq!(state.configure(), None);
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
    let refusal = Refusal::new(Field::Interval, "too fast".to_owned());
    let cases = [
        (
            Outcome::SavedUnsynced("disk".into()),
            Note::SavedUnsynced("disk".into()),
        ),
        (Outcome::Busy, Note::Busy),
        (Outcome::Failed("full".into()), Note::Failed("full".into())),
        (
            Outcome::Refused(refusal),
            Note::Refused(Field::Interval, "too fast".into()),
        ),
        (
            Outcome::Unavailable("no".into()),
            Note::Unavailable("no".into()),
        ),
        (Outcome::Configured, Note::Configured),
        (
            Outcome::CaptureEnded(CaptureEnd::TimedOut),
            Note::CaptureEnded(CaptureEnd::TimedOut),
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
            Note::Failed(_) | Note::Refused(..) | Note::Unavailable(_)
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
    assert!(!state.may_ask());
    assert_eq!(state.capture(Target::Key), None);
    let mut now = fields();
    now.tray_delay = 7;
    let again = state.agree(&now).unwrap().unwrap();
    let Ask::Save {
        settings, replace, ..
    } = &again.ask
    else {
        panic!("{again:?}");
    };
    assert!(*replace);
    assert_eq!(settings.tray_start_delay, 7);
    assert_eq!(state.question(), None);
    // The second question comes on top of the first agreement.
    let plain = Confirm::WithoutModifier(vec![Action::TogglePlayback]);
    answered(&mut state, again.id, Outcome::Confirm(plain));
    let last = state.agree(&now).unwrap().unwrap();
    let Ask::Save {
        replace,
        without_modifier,
        ..
    } = last.ask
    else {
        panic!("{last:?}");
    };
    assert!(replace);
    assert_eq!(without_modifier, [Action::TogglePlayback]);
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
    let mut portal = defaults();
    portal.input.hotkeys = RouteName::Portal;
    let mut shown = view(portal.clone(), None);
    assert!(idle(&shown) && desktop_hotkeys(&shown) && !routes_pending(&shown));
    shown.running.hotkeys = RouteName::Kernel;
    shown.activity = Activity::Clicking;
    assert!(!idle(&shown) && !desktop_hotkeys(&shown) && routes_pending(&shown));
}
