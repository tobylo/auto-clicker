//! The tray icon: shows the controller's view and sends it what the menu
//! asks for. It never changes the mode itself.

use auto_clicker_core::controller::{Activity, LISTED_RECORDINGS, Message, View};
use ksni::menu::{StandardItem, SubMenu};
use ksni::{MenuItem, ToolTip, TrayMethods};
use tokio::sync::{mpsc, watch};

/// The tray's state: the latest view, and where menu choices go.
pub struct Tray {
    view: View,
    send: mpsc::UnboundedSender<Message>,
}

/// Shows the tray icon and keeps it in step with the controller's view.
///
/// A missing tray host (Plasma's tray not running yet, or restarting) does not
/// fail this: the icon registers when the host appears. Fails only when the
/// session bus cannot be reached.
pub async fn show(
    mut views: watch::Receiver<View>,
    send: mpsc::UnboundedSender<Message>,
) -> Result<(), ksni::Error> {
    let view = views.borrow_and_update().clone();
    let handle = Tray { view, send }
        .assume_sni_available(true)
        .spawn()
        .await?;
    tokio::spawn(async move {
        // Ends when the controller is gone, or the tray service stopped.
        while views.changed().await.is_ok() {
            let view = views.borrow_and_update().clone();
            if handle.update(|tray| tray.view = view).await.is_none() {
                break;
            }
        }
    });
    Ok(())
}

impl Tray {
    fn send(&self, message: Message) {
        // The controller is gone only while the daemon exits.
        let _ = self.send.send(message);
    }
}

/// One menu item: its label, the message it sends (`None`: never enabled),
/// whether it is enabled and shows, and the items of its submenu, if it is
/// one.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    label: String,
    sends: Option<Message>,
    enabled: bool,
    visible: bool,
    children: Vec<Entry>,
}

impl Entry {
    /// An item that sends `sends` when enabled.
    fn item(label: &str, sends: Option<Message>, enabled: bool, visible: bool) -> Entry {
        Entry {
            label: label.to_owned(),
            enabled: enabled && sends.is_some(),
            sends,
            visible,
            children: Vec::new(),
        }
    }
}

/// The menu for `view`. The items are always the same, in the same places;
/// only which shows, which is enabled and what the Play submenu's places
/// hold changes. The host clicks an item by its place, so a click on a menu
/// opened before the state changed still sends what that item said, and a
/// click on a recording plays what that place holds now: the list changes
/// only when a recording is saved or the folder is changed by hand.
fn entries(view: &View) -> [Entry; 8] {
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
        Entry::item("Record", Some(Message::Record), idle, !activity.recording()),
        Entry::item(
            "Stop recording",
            Some(Message::StopRecording),
            true,
            activity.recording(),
        ),
        Entry {
            children: recordings(view),
            ..Entry::item("Play", None, false, !activity.playing())
        },
        Entry::item(
            "Stop playing",
            Some(Message::Stop),
            true,
            activity.playing(),
        ),
        // Settings come with M3.
        Entry::item("Settings", None, false, true),
        Entry::item("Quit", Some(Message::Quit), true, true),
    ]
}

/// The Play submenu: one place for each recording the controller may list,
/// newest first, the unused ones hidden, and a line that shows when there
/// are none. A recording can be chosen only while idle.
fn recordings(view: &View) -> Vec<Entry> {
    let idle = view.activity == Activity::Idle;
    let listed = (0..LISTED_RECORDINGS).map(|place| match view.recordings.get(place) {
        Some(listed) => Entry::item(
            &listed.label,
            Some(Message::Play(listed.file.clone())),
            idle,
            true,
        ),
        None => Entry::item("", None, false, false),
    });
    let none = Entry::item("No recordings yet", None, false, view.recordings.is_empty());
    listed.chain([none]).collect()
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

/// The menu item for `entry`: a submenu when it has children.
fn item(entry: Entry) -> MenuItem<Tray> {
    if entry.children.is_empty() {
        return StandardItem {
            label: entry.label,
            enabled: entry.enabled,
            visible: entry.visible,
            activate: Box::new(move |tray: &mut Tray| {
                if let Some(message) = entry.sends.clone() {
                    tray.send(message);
                }
            }),
            ..StandardItem::default()
        }
        .into();
    }
    SubMenu {
        label: entry.label,
        visible: entry.visible,
        submenu: entry.children.into_iter().map(item).collect(),
        ..SubMenu::default()
    }
    .into()
}

impl ksni::Tray for Tray {
    /// A left click opens the menu too; it never toggles clicking.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "auto-clicker".to_owned()
    }

    fn title(&self) -> String {
        self.view.title()
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
            title: escape(&self.view.title()),
            description: tooltip_body(&self.view),
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = entries(&self.view).into_iter().map(item).collect();
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
    use auto_clicker_core::controller::Listed;
    use auto_clicker_core::recording::FileName;

    use super::*;

    fn file(minute: usize) -> FileName {
        FileName::parse(&format!("2026-11-09_18-{minute:02}-00.000.json")).unwrap()
    }

    /// A view with this many recordings listed.
    fn view_listing(activity: Activity, recordings: usize) -> View {
        let config = Config::default();
        let recordings: Vec<Listed> = (0..recordings)
            .map(|minute| Listed {
                label: file(minute).label(),
                file: file(minute),
            })
            .collect();
        View {
            activity,
            settings: config.clicker,
            hotkey: config.hotkeys.toggle_clicking,
            record_hotkey: config.hotkeys.toggle_recording,
            play_hotkey: config.hotkeys.toggle_playback,
            playback: config.playback,
            recordings: Arc::from(recordings),
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
        entries(&view(activity))
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
        let rest = [("Play", false), ("Settings", false), ("Quit", true)];
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
            ("Settings", false),
            ("Quit", true),
        ];
        for busy in [PlayingIn(2), PlayWaitingForRelease, PlayOpening, Playing] {
            assert_eq!(shown(busy), labelled(playing), "{busy:?}");
        }
    }

    #[test]
    fn every_item_keeps_its_place_and_message() {
        use Activity::*;
        let messages = |activity| entries(&view(activity)).map(|entry| entry.sends);
        let expected = [
            Some(Message::Start),
            Some(Message::Stop),
            Some(Message::Record),
            Some(Message::StopRecording),
            None,
            Some(Message::Stop),
            None,
            Some(Message::Quit),
        ];
        for activity in [Idle, Clicking, Recording, RecordingIn(1), Playing] {
            assert_eq!(messages(activity), expected, "{activity:?}");
            let places = entries(&view(activity))[4].children.len();
            assert_eq!(places, LISTED_RECORDINGS + 1, "{activity:?}");
        }
    }

    /// The Play submenu's visible items: label, whether enabled, and what
    /// each sends.
    fn play_menu(view: &View) -> Vec<(String, bool, Option<Message>)> {
        let [_, _, _, _, play, ..] = entries(view);
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
                    (label, enabled, Some(Message::Play(file(minute))))
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
                MenuItem::SubMenu(menu) => activate(&menu.submenu, tray),
                _ => {}
            }
        }
    }

    #[test]
    fn the_tray_shows_the_view_and_its_menu_sends_messages() {
        use ksni::Tray as _;
        let (send, mut received) = mpsc::unbounded_channel();
        let mut tray = Tray {
            view: view_listing(Activity::Idle, 1),
            send,
        };
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
            item("Settings", false, true),
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
                // nothing, nor does Settings.
                Message::Play(file(0)),
                Message::Stop,
                Message::Quit,
                Message::MenuOpened
            ]
        );
    }

    #[test]
    fn tooltip_text_is_escaped() {
        let body = tooltip_body(&view(Activity::Idle));
        assert!(body.ends_with("<br/>a &lt;b&gt; &amp; c"), "{body}");
    }
}
