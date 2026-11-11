//! The tray icon: shows the controller's view and sends it what the menu
//! asks for. It never changes the mode itself.

use auto_clicker_core::controller::{Activity, LISTED_RECORDINGS, Message, View};
use auto_clicker_core::locale::Language;
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use ksni::{MenuItem, ToolTip, TrayMethods};
use tokio::sync::{mpsc, watch};

use crate::tray_text;

/// The tray's state: the latest view, and where menu choices go.
pub struct Tray {
    view: View,
    send: mpsc::UnboundedSender<Message>,
    /// Where a choice of Settings goes: the settings window's keeper.
    settings: mpsc::UnboundedSender<()>,
    /// The language the menu and the title line speak.
    language: Language,
    /// Flips whenever the list of recordings changes, which changes the
    /// menu's shape (see [`Tray::menu`](ksni::Tray::menu)).
    relisted: bool,
}

/// What a menu item does when chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Choice {
    /// Sends the controller this message.
    Send(Message),
    /// Opens the settings window, or raises the one open.
    Settings,
}

/// Shows the tray icon and keeps it in step with the controller's view.
///
/// A missing tray host (Plasma's tray not running yet, or restarting) does not
/// fail this: the icon registers when the host appears. Fails only when the
/// session bus cannot be reached.
pub async fn show(
    mut views: watch::Receiver<View>,
    send: mpsc::UnboundedSender<Message>,
    settings: mpsc::UnboundedSender<()>,
) -> Result<(), ksni::Error> {
    let view = views.borrow_and_update().clone();
    let handle = Tray::new(view, send, settings, Language::of_environment())
        .assume_sni_available(true)
        .spawn()
        .await?;
    tokio::spawn(async move {
        // Ends when the controller is gone, or the tray service stopped.
        while views.changed().await.is_ok() {
            let view = views.borrow_and_update().clone();
            if handle.update(|tray| tray.show(view)).await.is_none() {
                break;
            }
        }
    });
    Ok(())
}

impl Tray {
    fn new(
        view: View,
        send: mpsc::UnboundedSender<Message>,
        settings: mpsc::UnboundedSender<()>,
        language: Language,
    ) -> Tray {
        Tray {
            view,
            send,
            settings,
            language,
            relisted: false,
        }
    }

    /// Takes the controller's latest view.
    fn show(&mut self, view: View) {
        self.relisted ^= self.view.recordings != view.recordings;
        self.view = view;
    }

    fn send(&self, message: Message) {
        // The controller is gone only while the daemon exits.
        let _ = self.send.send(message);
    }

    fn choose(&self, choice: Choice) {
        match choice {
            Choice::Send(message) => self.send(message),
            // The keeper is gone only while the daemon exits.
            Choice::Settings => drop(self.settings.send(())),
        }
    }
}

/// One menu item: its label, the message it sends (`None`: never enabled),
/// whether it is enabled and shows, and the items of its submenu, if it is
/// one.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    label: String,
    sends: Option<Choice>,
    enabled: bool,
    visible: bool,
    children: Vec<Entry>,
    /// Whether it is checked, for an item that can be: a place in the Play
    /// submenu, checked for the recording the playback hotkey plays.
    checked: Option<bool>,
    /// The label is a name the user gave, shown as written: never
    /// translated.
    named: bool,
}

impl Entry {
    /// An item that sends `sends` when enabled.
    fn item(label: &str, sends: Option<Message>, enabled: bool, visible: bool) -> Entry {
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
fn entries(view: &View, relisted: bool) -> [Entry; 8] {
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

/// The Record item: off, saying why, where no recording can start.
fn record(view: &View) -> Entry {
    let (label, possible) = match &view.no_recording {
        Some(no_recording) => (no_recording.label(), false),
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

/// The icon for `activity`, by name from the icon theme.
fn icon_name(activity: Activity) -> &'static str {
    match activity {
        Activity::Clicking => "auto-clicker-clicking",
        Activity::Recording => "auto-clicker-recording",
        Activity::Playing => "auto-clicker-clicking",
        Activity::Idle
        | Activity::StartingIn(_)
        | Activity::WaitingForRelease
        | Activity::Opening
        | Activity::RecordingIn(_)
        | Activity::PlayingIn(_)
        | Activity::PlayWaitingForRelease
        | Activity::PlayOpening => "auto-clicker-idle",
    }
}

/// Tooltip text is markup on Plasma; config errors and device errors carry
/// text from outside, which must show as written.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn tooltip_body(view: &View) -> String {
    view.details()
        .iter()
        .map(|line| escape(line))
        .collect::<Vec<_>>()
        .join("<br/>")
}

/// The menu item for `entry`, labelled in `language`: a submenu when it
/// has children, an item with a check when it can be checked.
fn item(entry: Entry, language: Language) -> MenuItem<Tray> {
    let label = if entry.named {
        // An underscore marks an access key; two show as one.
        entry.label.replace('_', "__")
    } else {
        tray_text::label(&entry.label, language)
    };
    if let Some(checked) = entry.checked {
        return CheckmarkItem {
            label,
            enabled: entry.enabled,
            visible: entry.visible,
            checked,
            activate: activation(entry.sends),
            ..CheckmarkItem::default()
        }
        .into();
    }
    if entry.children.is_empty() {
        return StandardItem {
            label,
            enabled: entry.enabled,
            visible: entry.visible,
            activate: activation(entry.sends),
            ..StandardItem::default()
        }
        .into();
    }
    SubMenu {
        label,
        visible: entry.visible,
        submenu: entry
            .children
            .into_iter()
            .map(|child| item(child, language))
            .collect(),
        ..SubMenu::default()
    }
    .into()
}

/// What choosing an item that `sends` does.
fn activation(sends: Option<Choice>) -> Box<dyn Fn(&mut Tray) + Send> {
    Box::new(move |tray: &mut Tray| {
        if let Some(choice) = sends.clone() {
            tray.choose(choice);
        }
    })
}

impl ksni::Tray for Tray {
    /// A left click opens the menu too; it never toggles clicking.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "auto-clicker".to_owned()
    }

    fn title(&self) -> String {
        tray_text::title(&self.view, self.language)
    }

    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }

    fn icon_name(&self) -> String {
        icon_name(self.view.activity).to_owned()
    }

    /// Built for development, the icons come from the source tree; an
    /// installed build finds them in the hicolor theme.
    fn icon_theme_path(&self) -> String {
        if cfg!(debug_assertions) {
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons").to_owned()
        } else {
            String::new()
        }
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: self.icon_name(),
            icon_pixmap: Vec::new(),
            title: escape(&tray_text::title(&self.view, self.language)),
            description: tooltip_body(&self.view),
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let entries = entries(&self.view, self.relisted);
        let language = self.language;
        let mut items: Vec<MenuItem<Self>> = entries
            .into_iter()
            .map(|entry| item(entry, language))
            .collect();
        // Quit sits apart from the rest.
        items.insert(items.len().saturating_sub(1), MenuItem::Separator);
        items
    }

    /// Tells the controller the menu opened: a stop from it trims the trip
    /// to the tray from the click that opened it.
    fn menu_about_to_show(&mut self) {
        self.send(Message::MenuOpened);
    }

    /// Keeps running when the tray host goes away; the icon registers again
    /// when it comes back, and hotkeys keep working meanwhile.
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use auto_clicker_core::config::Config;
    use auto_clicker_core::controller::{Hotkey, Listed, NoRecording, Route, Routes};
    use auto_clicker_core::hotkeys::Unavailable;
    use auto_clicker_core::recording::FileName;

    use super::*;

    fn file(minute: usize) -> FileName {
        FileName::parse(&format!("2026-11-09_18-{minute:02}-00.000.json")).unwrap()
    }

    /// A view with this many recordings listed.
    fn view_listing(activity: Activity, recordings: usize) -> View {
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
            recordings: Arc::from(recordings),
            current: None,
            playing: None,
            notice: Some("a <b> & c".to_owned()),
            problems: Vec::new(),
        }
    }

    fn view(activity: Activity) -> View {
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

    fn labelled<const N: usize>(items: [(&str, bool); N]) -> Vec<(String, bool)> {
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
    fn the_play_menu_checks_the_recording_the_hotkey_plays_and_shows_names_as_written() {
        use ksni::Tray as _;
        let mut listing = view_listing(Activity::Idle, 3);
        let mut named: Vec<Listed> = listing.recordings.to_vec();
        named[0] = Listed::new(file(0), Some("Inställningar_1".to_owned()));
        named[1] = Listed::new(file(1), Some("Recording 2026-11-09 18:01".to_owned()));
        listing.recordings = Arc::from(named);
        listing.current = Some(file(1));
        let (send, _received) = mpsc::unbounded_channel();
        let tray = Tray::new(
            listing,
            send,
            mpsc::unbounded_channel().0,
            Language::Swedish,
        );
        let MenuItem::SubMenu(play) = &tray.menu()[4] else {
            panic!("Play is a submenu");
        };
        let places: Vec<(String, bool)> = play.submenu[..3]
            .iter()
            .map(|item| match item {
                MenuItem::Checkmark(item) => (item.label.clone(), item.checked),
                _ => panic!("a place can be checked"),
            })
            .collect();
        assert_eq!(
            places,
            [
                // A name is shown as written: never translated, and an
                // underscore is no access key.
                ("Inställningar__1 (2026-11-09 18:00:00)".to_owned(), false),
                // A recording with the name it was made with is labelled,
                // in the menu's language, as before.
                ("Inspelning 2026-11-09 18:01:00".to_owned(), true),
                ("Inspelning 2026-11-09 18:02:00".to_owned(), false),
            ]
        );
        // Every place can be checked, used or not; the line for none cannot.
        let kinds: Vec<bool> = play
            .submenu
            .iter()
            .map(|item| matches!(item, MenuItem::Checkmark(_)))
            .collect();
        let mut expected = vec![true; LISTED_RECORDINGS];
        expected.push(false);
        assert_eq!(kinds, expected);
    }

    #[test]
    fn the_play_menu_gives_way_to_stop_playing() {
        use ksni::Tray as _;
        let (send, _received) = mpsc::unbounded_channel();
        let tray = Tray::new(
            view(Activity::Playing),
            send,
            mpsc::unbounded_channel().0,
            Language::English,
        );
        let shown: Vec<(String, bool)> = tray
            .menu()
            .iter()
            .map(shown_item)
            .filter(|(label, ..)| label.starts_with("Play") || label.starts_with("Stop playing"))
            .map(|(label, _, visible, _)| (label, visible))
            .collect();
        assert_eq!(shown, labelled([("Play", false), ("Stop playing", true)]));
    }

    #[test]
    fn a_changed_list_changes_the_shape_of_the_play_menu_and_nothing_else_does() {
        use ksni::Tray as _;
        let (send, _received) = mpsc::unbounded_channel();
        let mut tray = Tray::new(
            view_listing(Activity::Idle, 10),
            send,
            mpsc::unbounded_channel().0,
            Language::English,
        );
        let places = |tray: &Tray| match &tray.menu()[4] {
            MenuItem::SubMenu(play) => play.submenu.len(),
            _ => 0,
        };
        let full = places(&tray);
        // The state and the notice change: the same shape.
        tray.show(view_listing(Activity::Clicking, 10));
        assert_eq!(places(&tray), full);
        // Another recording in every place, as after a save with ten
        // listed: a different shape, each time.
        let mut shifted = view_listing(Activity::Idle, 11);
        shifted.recordings = Arc::from(&shifted.recordings[1..]);
        tray.show(shifted.clone());
        assert_eq!(places(&tray), full + 1);
        tray.show(shifted);
        assert_eq!(places(&tray), full + 1);
        tray.show(view_listing(Activity::Idle, 10));
        assert_eq!(places(&tray), full);
    }

    #[test]
    fn the_icon_shows_clicking_playing_and_recording() {
        use Activity::*;
        assert_eq!(icon_name(Clicking), "auto-clicker-clicking");
        assert_eq!(icon_name(Playing), "auto-clicker-clicking");
        assert_eq!(icon_name(Recording), "auto-clicker-recording");
        for waiting in [
            Idle,
            StartingIn(1),
            WaitingForRelease,
            Opening,
            RecordingIn(1),
            PlayingIn(1),
            PlayWaitingForRelease,
            PlayOpening,
        ] {
            assert_eq!(icon_name(waiting), "auto-clicker-idle", "{waiting:?}");
        }
    }

    /// A menu item's label, whether it is enabled and shows, and its
    /// submenu's items.
    type Shown = (String, bool, bool, Vec<(String, bool, bool)>);

    fn standard(item: &MenuItem<Tray>) -> (String, bool, bool) {
        match item {
            MenuItem::Standard(item) => (item.label.clone(), item.enabled, item.visible),
            MenuItem::Checkmark(item) => (item.label.clone(), item.enabled, item.visible),
            _ => ("-".to_owned(), false, true),
        }
    }

    fn shown_item(item: &MenuItem<Tray>) -> Shown {
        match item {
            MenuItem::SubMenu(menu) => (
                menu.label.clone(),
                menu.enabled,
                menu.visible,
                menu.submenu.iter().map(standard).collect(),
            ),
            item => {
                let (label, enabled, visible) = standard(item);
                (label, enabled, visible, Vec::new())
            }
        }
    }

    /// Clicks every item, in menu order, submenus included.
    fn activate(items: &[MenuItem<Tray>], tray: &mut Tray) {
        for item in items {
            match item {
                MenuItem::Standard(item) => (item.activate)(tray),
                MenuItem::Checkmark(item) => (item.activate)(tray),
                MenuItem::SubMenu(menu) => activate(&menu.submenu, tray),
                _ => {}
            }
        }
    }

    #[test]
    fn the_tray_shows_the_view_and_its_menu_sends_messages() {
        use ksni::Tray as _;
        let (send, mut received) = mpsc::unbounded_channel();
        let (settings, mut opened) = mpsc::unbounded_channel();
        let mut tray = Tray::new(
            view_listing(Activity::Idle, 1),
            send,
            settings,
            Language::English,
        );
        assert_eq!(tray.id(), "auto-clicker");
        assert_eq!(tray.title(), "Auto clicker: idle");
        assert_eq!(tray.icon_name(), "auto-clicker-idle");
        // The source tree's icons in a development build, the theme's
        // otherwise.
        let path = tray.icon_theme_path();
        if cfg!(debug_assertions) {
            assert!(path.ends_with("/data/icons"), "{path}");
        } else {
            assert_eq!(path, "");
        }
        assert_eq!(tray.tool_tip().title, "Auto clicker: idle");
        assert!(tray.watcher_offline(ksni::OfflineReason::No));
        let items = tray.menu();
        let shown: Vec<Shown> = items.iter().map(shown_item).collect();
        let mut play_items = vec![("Recording 2026-11-09 18:00:00".to_owned(), true, true)];
        play_items.extend((1..LISTED_RECORDINGS).map(|_| (String::new(), false, false)));
        play_items.push(("No recordings yet".to_owned(), false, false));
        let item = |label: &str, on, shows| (label.to_owned(), on, shows, Vec::new());
        let expected = [
            item("Start clicking", true, true),
            item("Stop clicking", true, false),
            item("Record", true, true),
            item("Stop recording", true, false),
            ("Play".to_owned(), true, true, play_items),
            item("Stop playing", true, false),
            item("Settings", true, true),
            item("-", false, true),
            item("Quit", true, true),
        ];
        assert_eq!(shown, expected);
        activate(&items, &mut tray);
        tray.menu_about_to_show();
        let sent: Vec<Message> = std::iter::from_fn(|| received.try_recv().ok()).collect();
        assert_eq!(
            sent,
            [
                Message::Start,
                Message::Stop,
                Message::Record,
                Message::StopRecording,
                // The Play menu's unused places and its line for none send
                // nothing, and Settings goes to the window's keeper.
                Message::Play(file(0)),
                Message::Stop,
                Message::Quit,
                Message::MenuOpened
            ]
        );
        assert_eq!(opened.try_recv(), Ok(()));
        assert!(opened.try_recv().is_err());
    }

    #[test]
    fn the_menu_and_title_speak_swedish_in_a_swedish_locale() {
        use ksni::Tray as _;
        let (send, _received) = mpsc::unbounded_channel();
        let tray = Tray::new(
            view_listing(Activity::Idle, 1),
            send,
            mpsc::unbounded_channel().0,
            Language::Swedish,
        );
        assert_eq!(tray.title(), "Autoklickare: vilar");
        let labels: Vec<String> = tray.menu().iter().map(|item| shown_item(item).0).collect();
        assert_eq!(labels[0], "Börja klicka");
        assert_eq!(labels[6], "Inställningar");
        assert_eq!(labels[8], "Avsluta");
    }

    #[test]
    fn record_is_off_and_says_why_where_no_recording_can_start() {
        use Activity::*;
        let reasons = [
            NoRecording::DesktopHotkeys,
            NoRecording::Keyboards(Unavailable::NoPermission),
            NoRecording::Keyboards(Unavailable::WatchFailed("gone".to_owned())),
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
    fn the_tooltip_gives_the_reason_and_the_fix_escaped_like_every_line() {
        let shown = View {
            no_recording: Some(NoRecording::DesktopHotkeys),
            notice: Some("Not recording: <why>".to_owned()),
            ..view(Activity::Idle)
        };
        let lines: Vec<String> = tooltip_body(&shown)
            .split("<br/>")
            .map(str::to_owned)
            .collect();
        assert_eq!(
            lines[2],
            "Recording is unavailable with the desktop's shortcuts: nothing reads the \
             keyboards and mice. It needs the input group, and hotkeys = \"auto\" or \
             \"kernel\" under [input] in the config."
        );
        assert!(
            !lines.iter().any(|line| line.contains("starts recording")),
            "{lines:?}"
        );
        assert_eq!(
            lines.last().map(String::as_str),
            Some("Not recording: &lt;why&gt;")
        );
    }

    #[test]
    fn the_tooltip_names_the_route_of_each_job() {
        let cases = [
            (
                Route::Kernel,
                Route::Kernel,
                "Clicks through /dev/uinput, hotkeys from the keyboards",
            ),
            (
                Route::Kernel,
                Route::Portal,
                "Clicks through /dev/uinput, hotkeys from the desktop's shortcuts",
            ),
            (
                Route::Portal,
                Route::Kernel,
                "Clicks through the desktop's remote input, hotkeys from the keyboards",
            ),
            (
                Route::Portal,
                Route::Portal,
                "Clicks through the desktop's remote input, hotkeys from the desktop's \
                 shortcuts",
            ),
        ];
        for (output, hotkeys, line) in cases {
            let shown = View {
                routes: Routes { output, hotkeys },
                ..view(Activity::Idle)
            };
            let body = tooltip_body(&shown);
            let lines: Vec<&str> = body.split("<br/>").collect();
            // After the hotkeys, before the notice, which stays escaped.
            assert_eq!(lines[4], line, "{body}");
            assert_eq!(lines[5], "a &lt;b&gt; &amp; c", "{body}");
        }
    }

    #[test]
    fn tooltip_text_is_escaped() {
        let body = tooltip_body(&view(Activity::Idle));
        assert!(body.ends_with("<br/>a &lt;b&gt; &amp; c"), "{body}");
    }
}
