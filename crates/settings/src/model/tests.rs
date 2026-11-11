use auto_clicker_core::config::Config;
use auto_clicker_core::hotkeys::Binding;

use super::*;

fn draft(change: impl FnOnce(&mut Draft)) -> Draft {
    let mut draft = Draft::from(&Config::default());
    change(&mut draft);
    draft
}

/// Drafts away from the defaults in every field the window shows.
fn drafts() -> Vec<Draft> {
    vec![
        draft(|_| {}),
        draft(|d| {
            d.tray_start_delay = 3600;
            d.clicker.mode = ModeName::HoldKey;
            d.clicker.interval = IntervalDraft::PerSecond(7.5);
            d.clicker.jitter_percent = 50;
            d.clicker.button = ButtonName::Forward;
            d.clicker.key = 113;
            d.clicker.position = PositionDraft::Fixed { x: 0, y: 65535 };
            d.clicker.repeat = CountDraft::Times(12);
            d.input = InputDraft {
                output: RouteName::Portal,
                hotkeys: RouteName::Portal,
            };
            d.playback = PlaybackDraft {
                speed: 0.25,
                repeat: CountDraft::UntilStopped,
            };
        }),
        draft(|d| {
            d.clicker.mode = ModeName::DoubleClick;
            d.clicker.button = ButtonName::Back;
            d.clicker.interval = IntervalDraft::Ms(86_400_000);
            d.playback.speed = 4.0;
            d.playback.repeat = CountDraft::Times(3);
        }),
    ]
}

#[test]
fn the_fields_give_back_the_draft_they_show() {
    for shown in drafts() {
        let fields = Fields::from_draft(&shown);
        assert_eq!(fields.to_draft(&shown), Ok(shown.clone()), "{fields:?}");
    }
}

#[test]
fn hotkeys_and_the_key_come_from_the_captures_not_the_fields() {
    let shown = draft(|_| {});
    let captured = draft(|d| {
        d.clicker.key = 30;
        d.hotkeys.toggle_recording = "Ctrl+F9".to_owned();
    });
    let saved = Fields::from_draft(&shown).to_draft(&captured).unwrap();
    assert_eq!(saved.clicker.key, 30);
    assert_eq!(saved.hotkeys, captured.hotkeys);
}

#[test]
fn text_that_is_no_number_is_named() {
    let shown = draft(|_| {});
    let mut fields = Fields::from_draft(&shown);
    fields.interval_text = "ten".to_owned();
    assert_eq!(fields.to_draft(&shown), Err(NotANumber::Interval));
    fields.interval_text = " 250 ".to_owned();
    fields.speed_text = "fast".to_owned();
    assert_eq!(fields.to_draft(&shown), Err(NotANumber::Speed));
    fields.interval_unit = 1;
    fields.interval_text = "2.5".to_owned();
    fields.speed_text = "1.5".to_owned();
    let read = fields.to_draft(&shown).unwrap();
    assert_eq!(read.clicker.interval, IntervalDraft::PerSecond(2.5));
}

#[test]
fn a_choice_no_widget_gives_falls_back_and_numbers_stay_in_range() {
    let shown = draft(|_| {});
    let mut fields = Fields::from_draft(&shown);
    fields.mode = 9;
    fields.button = -1;
    fields.position_fixed = true;
    fields.position_x = 70_000;
    fields.position_y = -5;
    fields.jitter = -1;
    let read = fields.to_draft(&shown).unwrap();
    assert_eq!(read.clicker.mode, ModeName::Click);
    assert_eq!(read.clicker.button, ButtonName::Left);
    assert_eq!(
        read.clicker.position,
        PositionDraft::Fixed { x: 65535, y: 0 }
    );
    assert_eq!(read.clicker.jitter_percent, 0);
}

#[test]
fn hotkeys_and_keys_show_as_the_user_knows_them() {
    let bound = Hotkey::Bound(Binding::parse("ctrl+f9").unwrap());
    assert_eq!(hotkey_text(&bound, "-", "?"), "Ctrl+F9");
    assert_eq!(
        hotkey_text(&Hotkey::Desktop(Some(String::new())), "-", "?"),
        "-"
    );
    assert_eq!(
        hotkey_text(&Hotkey::Desktop(Some("Meta+P".to_owned())), "-", "?"),
        "Meta+P"
    );
    assert_eq!(hotkey_text(&Hotkey::Desktop(None), "-", "?"), "?");
    assert_eq!(key_text(57), "space");
    assert_eq!(key_text(113), "113");
}

#[test]
fn a_captured_hotkey_replaces_its_actions_only() {
    let hotkeys = draft(|_| {}).hotkeys;
    for action in 0..3 {
        let changed = with_hotkey(&hotkeys, action, "F13".to_owned());
        let texts = [
            &changed.toggle_clicking,
            &changed.toggle_recording,
            &changed.toggle_playback,
        ];
        let replaced: Vec<usize> = (0..3).filter(|i| texts[*i] == "F13").collect();
        assert_eq!(replaced, [usize::try_from(action).unwrap()]);
    }
}
