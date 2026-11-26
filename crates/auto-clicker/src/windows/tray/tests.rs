use std::sync::Arc;

use auto_clicker_core::controller::{Activity, Listed};

use super::*;
use crate::tray_menu::tests::{file, view, view_listing};

/// Each item's label, whether it is on, and its command.
fn items(menu: &[MenuEntry]) -> Vec<(String, bool, Option<u32>)> {
    menu.iter()
        .map(|entry| match entry {
            MenuEntry::Item {
                label,
                enabled,
                command,
                ..
            } => (label.clone(), *enabled, *command),
            MenuEntry::Submenu { label, .. } => (format!("{label} >"), true, None),
            MenuEntry::Separator => ("-".to_owned(), false, None),
        })
        .collect()
}

fn item(label: &str, enabled: bool, command: Option<u32>) -> (String, bool, Option<u32>) {
    (label.to_owned(), enabled, command)
}

#[test]
fn the_menu_is_linuxs_with_settings_off_until_the_window_comes() {
    let commands = Commands::default();
    let idle = content(&view(Activity::Idle), Language::English, 16, &commands);
    assert_eq!(
        items(&idle.menu),
        [
            item("Start clicking", true, Some(START)),
            item("Record", true, Some(RECORD)),
            item("Play >", true, None),
            item("Settings (not on Windows yet)", false, None),
            item("-", false, None),
            item("Quit", true, Some(QUIT)),
        ]
    );
    let clicking = content(&view(Activity::Clicking), Language::Swedish, 16, &commands);
    assert_eq!(
        items(&clicking.menu),
        [
            item("Sluta klicka", true, Some(STOP)),
            item("Spela in", false, Some(RECORD)),
            item("Spela upp >", true, None),
            item("Inställningar (finns inte på Windows än)", false, None),
            item("-", false, None),
            item("Avsluta", true, Some(QUIT)),
        ]
    );
    let playing = content(&view(Activity::Playing), Language::English, 16, &commands);
    assert!(items(&playing.menu).contains(&item("Stop playing", true, Some(STOP))));
}

#[test]
fn the_play_menu_lists_the_recordings_as_written_and_checks_the_current() {
    let commands = Commands::default();
    let mut listing = view_listing(Activity::Idle, 2);
    let mut named: Vec<Listed> = listing.recordings.to_vec();
    named[0] = Listed::new(file(0), Some("Rock & roll".to_owned()));
    listing.recordings = Arc::from(named);
    listing.current = Some(file(1));
    let shown = content(&listing, Language::Swedish, 16, &commands);
    let MenuEntry::Submenu { children, .. } = &shown.menu[2] else {
        panic!("Play is a submenu: {:?}", shown.menu);
    };
    let checked: Vec<Option<bool>> = children
        .iter()
        .map(|child| match child {
            MenuEntry::Item { checked, .. } => *checked,
            _ => None,
        })
        .collect();
    assert_eq!(
        items(children),
        [
            item(
                "Rock & roll (2026-11-09 18:00:00)",
                true,
                Some(FIRST_RECORDING)
            ),
            item(
                "Inspelning 2026-11-09 18:01:00",
                true,
                Some(FIRST_RECORDING + 1)
            ),
        ]
    );
    assert_eq!(checked, [Some(false), Some(true)]);
    let none = content(
        &view_listing(Activity::Idle, 0),
        Language::English,
        16,
        &commands,
    );
    let MenuEntry::Submenu { children, .. } = &none.menu[2] else {
        panic!("Play is a submenu");
    };
    assert_eq!(items(children), [item("No recordings yet", false, None)]);
}

#[test]
fn a_choice_sends_what_its_item_said_after_the_view_changed() {
    let commands = Commands::default();
    // Drawn while two recordings are listed.
    content(
        &view_listing(Activity::Idle, 2),
        Language::English,
        16,
        &commands,
    );
    // A new recording comes first: the others keep their commands.
    let mut shifted = view_listing(Activity::Idle, 3);
    let mut newer: Vec<Listed> = shifted.recordings.to_vec();
    newer.rotate_right(1);
    shifted.recordings = Arc::from(newer);
    content(&shifted, Language::English, 16, &commands);
    for (command, message) in [
        (START, Message::Start),
        (STOP, Message::Stop),
        (RECORD, Message::Record),
        (STOP_RECORDING, Message::StopRecording),
        (QUIT, Message::Quit),
        (FIRST_RECORDING, Message::Play(file(0))),
        (FIRST_RECORDING + 1, Message::Play(file(1))),
        (FIRST_RECORDING + 2, Message::Play(file(2))),
    ] {
        assert_eq!(commands.message(command), Some(message), "{command}");
    }
    // A command the menu never gave sends nothing.
    for unknown in [0, 6, 99, FIRST_RECORDING + 3, u32::MAX] {
        assert_eq!(commands.message(unknown), None, "{unknown}");
    }
}

#[test]
fn the_icon_title_and_lines_follow_the_view() {
    let commands = Commands::default();
    for activity in [Activity::Idle, Activity::StartingIn(3), Activity::Recording] {
        let shown = view(activity);
        let drawn = content(&shown, Language::English, 20, &commands);
        let image = tray_icon::image(activity, 20);
        assert_eq!((drawn.icon.side, drawn.icon.rgba), (image.side, image.rgba));
        assert_eq!(drawn.title, shown.title());
        assert_eq!(drawn.details, tray_text::details(&shown, Language::English));
        // Windows has one route of each kind, and names none.
        assert!(
            !drawn.details.contains(&shown.routes.line()),
            "{activity:?}"
        );
    }
}
