//! The tray's menu, the same on both systems: which items show, which are
//! on, what each says and what each sends. Each system's tray turns these
//! entries into its own menu; neither changes the mode itself.

use auto_clicker_core::controller::{Activity, LISTED_RECORDINGS, Message, View};
use auto_clicker_core::message::Problem;

/// What a menu item does when chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Sends the controller this message.
    Send(Message),
    /// Opens the settings window, or raises the one open.
    Settings,
}

/// One menu item: its label, the message it sends (`None`: never enabled),
/// whether it is enabled and shows, and the items of its submenu, if it is
/// one.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    pub sends: Option<Choice>,
    pub enabled: bool,
    pub visible: bool,
    pub children: Vec<Entry>,
    /// Whether it is checked, for an item that can be: a place in the Play
    /// submenu, checked for the recording the playback hotkey plays.
    pub checked: Option<bool>,
    /// The label is a name the user gave, shown as written: never
    /// translated.
    pub named: bool,
}

impl Entry {
    /// An item that sends `sends` when enabled.
    pub fn item(label: &str, sends: Option<Message>, enabled: bool, visible: bool) -> Entry {
        Entry {
            label: label.to_owned(),
            enabled: enabled && sends.is_some(),
            sends: sends.map(Choice::Send),
            visible,
            children: Vec::new(),
            checked: None,
            named: false,
        }
    }

    /// The Settings item: always there, and always enabled.
    fn settings() -> Entry {
        Entry {
            label: "Settings".to_owned(),
            sends: Some(Choice::Settings),
            enabled: true,
            visible: true,
            children: Vec::new(),
            checked: None,
            named: false,
        }
    }
}

/// The menu for `view`. The items are always the same, in the same places;
/// only which shows, which is enabled and what the Play submenu's places
/// hold changes. The host clicks an item by its place, so a click on a menu
/// opened before the state changed still sends what that item said.
///
/// The Play submenu's places are the exception, since what a place holds
/// changes with the list: `relisted` adds one hidden item to it, so the
/// menu's shape differs from before whenever the list does. The host is then
/// told to lay the menu out again, and a click it sends for the old layout
/// is refused rather than playing the recording that took the place.
pub fn entries(view: &View, relisted: bool) -> [Entry; 8] {
    let activity = view.activity;
    let idle = activity == Activity::Idle;
    [
        Entry::item(
            "Start clicking",
            Some(Message::Start),
            idle,
            !activity.clicking(),
        ),
        Entry::item(
            "Stop clicking",
            Some(Message::Stop),
            true,
            activity.clicking(),
        ),
        record(view),
        Entry::item(
            "Stop recording",
            Some(Message::StopRecording),
            true,
            activity.recording(),
        ),
        Entry {
            children: recordings(view, relisted),
            ..Entry::item("Play", None, false, !activity.playing())
        },
        Entry::item(
            "Stop playing",
            Some(Message::Stop),
            true,
            activity.playing(),
        ),
        Entry::settings(),
        Entry::item("Quit", Some(Message::Quit), true, true),
    ]
}

/// The Record item: off, saying why, where no recording can start; on,
/// saying so, where recordings keep movement only.
fn record(view: &View) -> Entry {
    let movement_only = view
        .problems
        .iter()
        .any(|problem| matches!(problem, Problem::NoPositions(_)));
    let (label, possible) = match &view.no_recording {
        Some(no_recording) => (no_recording.label(), false),
        None if movement_only => ("Record (movement only)", true),
        None => ("Record", true),
    };
    let idle = view.activity == Activity::Idle;
    Entry::item(
        label,
        Some(Message::Record),
        idle && possible,
        !view.activity.recording(),
    )
}

/// The Play submenu: one place for each recording the tray lists, newest
/// first, the unused ones hidden, and a line that shows when there are
/// none. The recording the playback hotkey plays is checked; a place is
/// always an item that can be, so a check never changes the menu's shape.
/// A recording can be chosen only while idle. `relisted` adds a hidden item
/// at the end.
fn recordings(view: &View, relisted: bool) -> Vec<Entry> {
    let idle = view.activity == Activity::Idle;
    let listed = (0..LISTED_RECORDINGS).map(|place| match view.recordings.get(place) {
        Some(listed) => Entry {
            checked: Some(view.current.as_ref() == Some(&listed.file)),
            named: listed.label != listed.file.label(),
            ..Entry::item(
                &listed.label,
                Some(Message::Play(listed.file.clone())),
                idle,
                true,
            )
        },
        None => Entry {
            checked: Some(false),
            ..Entry::item("", None, false, false)
        },
    });
    let none = Entry::item("No recordings yet", None, false, view.recordings.is_empty());
    let marker = relisted.then(|| Entry::item("", None, false, false));
    listed.chain([none]).chain(marker).collect()
}

#[cfg(test)]
pub mod tests {
    use std::sync::Arc;

    use auto_clicker_core::config::Config;
    use auto_clicker_core::controller::{Hotkey, Listed, NoRecording, Routes};
    use auto_clicker_core::hotkeys::Unavailable;
    use auto_clicker_core::locale::Language;
    use auto_clicker_core::message::{Notice, Titled};
    use auto_clicker_core::recording::FileName;

    use super::*;
    use crate::tray_text;

    pub fn file(minute: usize) -> FileName {
        FileName::parse(&format!("2026-11-09_18-{minute:02}-00.000.json")).unwrap()
    }

    /// A view with this many recordings listed.
    pub fn view_listing(activity: Activity, recordings: usize) -> View {
        let config = Config::default();
        let recordings: Vec<Listed> = (0..recordings)
            .map(|minute| Listed::new(file(minute), None))
            .collect();
        View {
            activity,
            routes: Routes::default(),
            settings: config.clicker,
            hotkey: Hotkey::Bound(config.hotkeys.toggle_clicking),
            record_hotkey: Hotkey::Bound(config.hotkeys.toggle_recording),
            no_recording: None,
            play_hotkey: Hotkey::Bound(config.hotkeys.toggle_playback),
            playback: config.playback,
            on_screen: config.on_screen,
            recordings: Arc::from(recordings),
            current: None,
            playing: None,
            notice: Some(Notice::Finished(Some(Titled::new(
                file(0),
                Some("a <b> & c"),
            )))),
            problems: Vec::new(),
        }
    }

    pub fn view(activity: Activity) -> View {
        view_listing(activity, 2)
    }

    /// The visible items: label, and whether enabled.
    fn shown(activity: Activity) -> Vec<(String, bool)> {
        entries(&view(activity), false)
            .into_iter()
            .filter(|entry| entry.visible)
            .map(|entry| (entry.label, entry.enabled))
            .collect()
    }

    pub fn labelled<const N: usize>(items: [(&str, bool); N]) -> Vec<(String, bool)> {
        items
            .into_iter()
            .map(|(label, enabled)| (label.to_owned(), enabled))
            .collect()
    }

    #[test]
    fn the_menu_offers_what_each_state_allows() {
        use Activity::*;
        // Play is a submenu, enabled through its items.
        let rest = [("Play", false), ("Settings", true), ("Quit", true)];
        let idle = [("Start clicking", true), ("Record", true)];
        assert_eq!(
            shown(Idle),
            labelled(idle)
                .into_iter()
                .chain(labelled(rest))
                .collect::<Vec<_>>()
        );
        let clicking = [("Stop clicking", true), ("Record", false)];
        for busy in [StartingIn(3), WaitingForRelease, Opening, Clicking] {
            let expected: Vec<_> = labelled(clicking)
                .into_iter()
                .chain(labelled(rest))
                .collect();
            assert_eq!(shown(busy), expected, "{busy:?}");
        }
        let recording = [("Start clicking", false), ("Stop recording", true)];
        for busy in [RecordingIn(2), Recording] {
            let expected: Vec<_> = labelled(recording)
                .into_iter()
                .chain(labelled(rest))
                .collect();
            assert_eq!(shown(busy), expected, "{busy:?}");
        }
        let playing = [
            ("Start clicking", false),
            ("Record", false),
            ("Stop playing", true),
            ("Settings", true),
            ("Quit", true),
        ];
        for busy in [PlayingIn(2), PlayWaitingForRelease, PlayOpening, Playing] {
            assert_eq!(shown(busy), labelled(playing), "{busy:?}");
        }
    }

    #[test]
    fn every_item_keeps_its_place_and_message() {
        use Activity::*;
        let messages = |activity| entries(&view(activity), false).map(|entry| entry.sends);
        let send = |message| Some(Choice::Send(message));
        let expected = [
            send(Message::Start),
            send(Message::Stop),
            send(Message::Record),
            send(Message::StopRecording),
            None,
            send(Message::Stop),
            Some(Choice::Settings),
            send(Message::Quit),
        ];
        for activity in [Idle, Clicking, Recording, RecordingIn(1), Playing] {
            assert_eq!(messages(activity), expected, "{activity:?}");
            let places = entries(&view(activity), false)[4].children.len();
            assert_eq!(places, LISTED_RECORDINGS + 1, "{activity:?}");
        }
    }

    /// The Play submenu's visible items: label, whether enabled, and what
    /// each sends.
    fn play_menu(view: &View) -> Vec<(String, bool, Option<Choice>)> {
        let [_, _, _, _, play, ..] = entries(view, false);
        play.children
            .into_iter()
            .filter(|entry| entry.visible)
            .map(|entry| (entry.label, entry.enabled, entry.sends))
            .collect()
    }

    #[test]
    fn the_play_menu_lists_the_recordings_newest_first_or_says_there_are_none() {
        let listed = |enabled| {
            (0..2)
                .map(|minute| {
                    let label = format!("Recording 2026-11-09 18:{minute:02}:00");
                    (
                        label,
                        enabled,
                        Some(Choice::Send(Message::Play(file(minute)))),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(play_menu(&view(Activity::Idle)), listed(true));
        // A recording can be chosen only while idle.
        for busy in [
            Activity::Clicking,
            Activity::Recording,
            Activity::StartingIn(1),
        ] {
            assert_eq!(play_menu(&view(busy)), listed(false), "{busy:?}");
        }
        assert_eq!(
            play_menu(&view_listing(Activity::Idle, 0)),
            [("No recordings yet".to_owned(), false, None)]
        );
        assert_eq!(play_menu(&view_listing(Activity::Idle, 10)).len(), 10);
    }

    #[test]
    fn record_is_off_and_says_why_where_no_recording_can_start() {
        use Activity::*;
        let reasons = [
            NoRecording::DesktopHotkeys,
            NoRecording::Keyboards(Unavailable::NoPermission),
            NoRecording::Keyboards(Unavailable::WatchFailed("gone".to_owned().into())),
        ];
        for no_recording in reasons {
            let label = no_recording.label();
            for activity in [Idle, Clicking, Playing] {
                let shown = View {
                    no_recording: Some(no_recording.clone()),
                    ..view(activity)
                };
                let [_, _, record, stop, ..] = entries(&shown, false);
                assert_eq!(
                    (record.label.as_str(), record.enabled, record.visible),
                    (label, false, true),
                    "{activity:?}"
                );
                assert!(!stop.visible, "{activity:?}");
            }
            // A recording the keyboards went away under still stops.
            let recording = View {
                no_recording: Some(no_recording),
                ..view(Recording)
            };
            let [_, _, record, stop, ..] = entries(&recording, false);
            assert!(!record.visible);
            assert_eq!(
                (stop.label.as_str(), stop.enabled, stop.visible),
                ("Stop recording", true, true)
            );
        }
    }

    #[test]
    fn record_stays_on_and_says_movement_only_where_positions_are_missing() {
        use auto_clicker_core::message::{NoPointer, NoPositions};
        let missing = Problem::NoPositions(NoPositions::Pointer(NoPointer::NoSource));
        let shown = View {
            problems: vec![missing.clone()],
            ..view(Activity::Idle)
        };
        let [_, _, record, ..] = entries(&shown, false);
        assert_eq!(
            (record.label.as_str(), record.enabled, record.visible),
            ("Record (movement only)", true, true)
        );
        assert_eq!(
            tray_text::label(&record.label, Language::Swedish),
            "Spela in (bara rörelser)"
        );
        // Where no recording can start, what it needs is said instead.
        let blocked = View {
            no_recording: Some(NoRecording::DesktopHotkeys),
            ..shown
        };
        let [_, _, record, ..] = entries(&blocked, false);
        assert_eq!(record.label, NoRecording::DesktopHotkeys.label());
    }
}
