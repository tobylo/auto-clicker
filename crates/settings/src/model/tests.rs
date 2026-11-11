use auto_clicker_core::config::Config;
use auto_clicker_core::hotkeys::Binding;

use super::*;

fn draft(change: impl FnOnce(&mut Draft)) -> Draft {
    let mut draft = Draft::from(&Config::default());
    change(&mut draft);
    draft
}

/// Drafts away from the defaults in every field the window shows, the
/// counts at the largest the config takes.
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
            d.clicker.repeat = CountDraft::Times(u64::from(u32::MAX));
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
            d.clicker.key = 42;
            d.playback.speed = 4.0;
            d.playback.repeat = CountDraft::Times(u64::from(u32::MAX));
        }),
    ]
}

#[test]
fn the_fields_give_back_the_draft_they_show_in_either_language() {
    for decimal in ['.', ','] {
        for shown in drafts() {
            let fields = Fields::from_draft(&shown, decimal);
            assert_eq!(
                fields.to_draft(&shown.hotkeys),
                Ok(shown.clone()),
                "{fields:?}"
            );
        }
    }
}

#[test]
fn decimals_show_with_the_languages_separator_and_read_with_either() {
    let shown = draft(|d| d.playback.speed = 1.5);
    assert_eq!(Fields::from_draft(&shown, ',').speed_text, "1,5");
    assert_eq!(Fields::from_draft(&shown, '.').speed_text, "1.5");
    let mut fields = Fields::from_draft(&shown, ',');
    for written in ["0,75", "0.75", " 0,75 "] {
        fields.speed_text = written.to_owned();
        assert_eq!(
            fields.to_draft(&shown.hotkeys).unwrap().playback.speed,
            0.75,
            "{written}"
        );
    }
}

#[test]
fn the_hotkeys_come_from_the_captures_not_the_fields() {
    let shown = draft(|_| {});
    let captured = draft(|d| d.hotkeys.toggle_recording = "Ctrl+F9".to_owned()).hotkeys;
    let saved = Fields::from_draft(&shown, '.').to_draft(&captured).unwrap();
    assert_eq!(saved.hotkeys, captured);
}

#[test]
fn the_key_is_a_name_in_any_case_or_a_code() {
    let shown = draft(|_| {});
    let mut fields = Fields::from_draft(&shown, '.');
    assert_eq!(fields.key_text, "space");
    for (written, code) in [("F13", 183), ("leftshift", 42), (" 113 ", 113), ("W", 17)] {
        fields.key_text = written.to_owned();
        assert_eq!(
            fields.to_draft(&shown.hotkeys).unwrap().clicker.key,
            code,
            "{written}"
        );
    }
    for unreadable in ["", "no such key", "70000", "-1"] {
        fields.key_text = unreadable.to_owned();
        assert_eq!(
            fields.to_draft(&shown.hotkeys),
            Err(Unreadable::Key),
            "{unreadable}"
        );
    }
}

/// How a case changes the fields, and the field it makes unreadable.
type Case = (fn(&mut Fields), Unreadable);

#[test]
fn text_that_cannot_be_read_is_named() {
    let shown = draft(|_| {});
    let base = Fields::from_draft(&shown, '.');
    let cases: [Case; 7] = [
        (|f| f.interval_text = "ten".to_owned(), Unreadable::Interval),
        (|f| f.interval_text = "2.5".to_owned(), Unreadable::Interval),
        (
            |f| {
                f.interval_unit = 1;
                f.interval_text = "NaN".to_owned();
            },
            Unreadable::Interval,
        ),
        (|f| f.speed_text = "fast".to_owned(), Unreadable::Speed),
        (|f| f.speed_text = "1e999".to_owned(), Unreadable::Speed),
        (
            |f| {
                f.repeat_forever = false;
                f.repeat_text = "lots".to_owned();
            },
            Unreadable::Repeat,
        ),
        (
            |f| {
                f.plays_forever = false;
                f.plays_text = "-3".to_owned();
            },
            Unreadable::Plays,
        ),
    ];
    for (change, field) in cases {
        let mut fields = base.clone();
        change(&mut fields);
        assert_eq!(fields.to_draft(&shown.hotkeys), Err(field), "{fields:?}");
    }
}

#[test]
fn a_count_beyond_what_the_config_takes_goes_to_the_daemon_to_refuse() {
    let shown = draft(|_| {});
    let mut fields = Fields::from_draft(&shown, '.');
    fields.repeat_forever = false;
    fields.repeat_text = "5000000000".to_owned();
    let read = fields.to_draft(&shown.hotkeys).unwrap();
    assert_eq!(read.clicker.repeat, CountDraft::Times(5_000_000_000));
}

#[test]
fn a_choice_no_widget_gives_falls_back_and_numbers_stay_in_range() {
    let shown = draft(|_| {});
    let mut fields = Fields::from_draft(&shown, '.');
    fields.mode = 9;
    fields.button = -1;
    fields.position_fixed = true;
    fields.position_x = 70_000;
    fields.position_y = -5;
    fields.jitter = -1;
    let read = fields.to_draft(&shown.hotkeys).unwrap();
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
