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

/// A state that took the default settings.
fn started() -> State {
    let mut state = State::default();
    let shown = state.view(&view(defaults(), None));
    assert_eq!(shown.fields, Some(Fields::from_draft(&defaults())));
    state
}

fn answered(state: &mut State, id: u64, outcome: Outcome) -> Shown {
    state.view(&view(defaults(), Some(Answer { id, outcome })))
}

#[test]
fn the_fields_fill_when_the_saved_settings_change_and_only_then() {
    let mut state = started();
    assert_eq!(state.view(&view(defaults(), None)), Shown::default());
    let mut changed = defaults();
    changed.tray_start_delay = 9;
    let shown = state.view(&view(changed.clone(), None));
    assert_eq!(shown.fields, Some(Fields::from_draft(&changed)));
}

#[test]
fn nothing_is_saved_before_the_settings_came() {
    let mut state = State::default();
    assert_eq!(state.save(&Fields::from_draft(&defaults())), None);
    assert!(!state.waiting());
}

#[test]
fn a_save_waits_for_its_own_answer() {
    let mut state = started();
    let request = state
        .save(&Fields::from_draft(&defaults()))
        .unwrap()
        .unwrap();
    assert!(state.waiting());
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
    assert!(!state.waiting());
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
        let request = state.configure();
        assert_eq!(
            answered(&mut state, request.id, outcome).note,
            Some(note.clone())
        );
        assert_eq!(
            note.is_error(),
            matches!(
                note,
                Note::Failed(_) | Note::Refused(..) | Note::Unavailable(_)
            )
        );
    }
}

#[test]
fn a_question_is_asked_and_a_yes_sends_the_save_again_agreeing() {
    let mut state = started();
    let first = state
        .save(&Fields::from_draft(&defaults()))
        .unwrap()
        .unwrap();
    let ask = Confirm::Replace(Stale::Changed);
    assert_eq!(
        answered(&mut state, first.id, Outcome::Confirm(ask)).note,
        None
    );
    assert_eq!(state.question(), Some(&Question::Replace(Stale::Changed)));
    let again = state.agree().unwrap();
    assert!(
        matches!(again.ask, Ask::Save { replace: true, .. }),
        "{again:?}"
    );
    assert_eq!(state.question(), None);
    let plain = Confirm::WithoutModifier(vec![Action::TogglePlayback]);
    answered(&mut state, again.id, Outcome::Confirm(plain));
    let last = state.agree().unwrap();
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
fn a_no_sends_nothing() {
    let mut state = started();
    let first = state
        .save(&Fields::from_draft(&defaults()))
        .unwrap()
        .unwrap();
    answered(
        &mut state,
        first.id,
        Outcome::Confirm(Confirm::Replace(Stale::Unusable)),
    );
    state.decline();
    assert_eq!(state.question(), None);
    assert_eq!(state.agree(), None);
}

#[test]
fn a_captured_hotkey_or_key_is_kept_for_the_next_save() {
    let mut state = started();
    let capture = state.capture(Target::Hotkey(1));
    assert_eq!(capture.ask, Ask::Capture(Capture::Hotkey));
    let ctrl_f9 = Binding::parse("Ctrl+F9").unwrap();
    assert_eq!(
        answered(&mut state, capture.id, Outcome::CapturedHotkey(ctrl_f9)).note,
        None
    );
    let key = state.capture(Target::Key);
    assert_eq!(key.ask, Ask::Capture(Capture::Key));
    answered(&mut state, key.id, Outcome::CapturedKey(30));
    let save = state
        .save(&Fields::from_draft(&defaults()))
        .unwrap()
        .unwrap();
    let Ask::Save { settings, .. } = save.ask else {
        panic!("{save:?}");
    };
    assert_eq!(settings.hotkeys.toggle_recording, "Ctrl+F9");
    assert_eq!(settings.clicker.key, 30);
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
