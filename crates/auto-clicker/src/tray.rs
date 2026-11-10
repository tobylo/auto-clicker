//! The tray icon: shows the controller's view and sends it what the menu
//! asks for. It never changes the mode itself.

use auto_clicker_core::controller::{Activity, Message, View};
use ksni::menu::StandardItem;
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
/// and whether it is enabled and shows.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    label: &'static str,
    sends: Option<Message>,
    enabled: bool,
    visible: bool,
}

/// The menu for `view`. The items are always the same, in the same places,
/// each with one fixed message; only which shows and which is enabled
/// changes. The host clicks an item by its place, so a click on a menu
/// opened before the state changed still sends what that item said.
fn entries(view: &View) -> [Entry; 7] {
    let activity = view.activity;
    let idle = activity == Activity::Idle;
    let item = |label, sends: Option<Message>, enabled: bool, visible| Entry {
        label,
        enabled: enabled && sends.is_some(),
        sends,
        visible,
    };
    [
        item(
            "Start clicking",
            Some(Message::Start),
            idle,
            !activity.clicking(),
        ),
        item(
            "Stop clicking",
            Some(Message::Stop),
            true,
            activity.clicking(),
        ),
        item("Record", Some(Message::Record), idle, !activity.recording()),
        item(
            "Stop recording",
            Some(Message::StopRecording),
            true,
            activity.recording(),
        ),
        // Playback comes with R02, settings with M3.
        item("Play", None, false, true),
        item("Settings", None, false, true),
        item("Quit", Some(Message::Quit), true, true),
    ]
}

/// The icon for `activity`, by name from the icon theme.
fn icon_name(activity: Activity) -> &'static str {
    match activity {
        Activity::Clicking => "auto-clicker-clicking",
        Activity::Recording => "auto-clicker-recording",
        Activity::Idle
        | Activity::StartingIn(_)
        | Activity::WaitingForRelease
        | Activity::RecordingIn(_) => "auto-clicker-idle",
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
        let mut items: Vec<MenuItem<Self>> = entries(&self.view)
            .into_iter()
            .map(|entry| {
                StandardItem {
                    label: entry.label.to_owned(),
                    enabled: entry.enabled,
                    visible: entry.visible,
                    activate: Box::new(move |tray: &mut Self| {
                        if let Some(message) = entry.sends.clone() {
                            tray.send(message);
                        }
                    }),
                    ..StandardItem::default()
                }
                .into()
            })
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
    use auto_clicker_core::config::Config;

    use super::*;

    fn view(activity: Activity) -> View {
        let config = Config::default();
        View {
            activity,
            settings: config.clicker,
            hotkey: config.hotkeys.toggle_clicking,
            record_hotkey: config.hotkeys.toggle_recording,
            notice: Some("a <b> & c".to_owned()),
            problems: Vec::new(),
        }
    }

    /// The visible items: label, and whether enabled.
    fn shown(activity: Activity) -> Vec<(&'static str, bool)> {
        entries(&view(activity))
            .into_iter()
            .filter(|entry| entry.visible)
            .map(|entry| (entry.label, entry.enabled))
            .collect()
    }

    #[test]
    fn the_menu_offers_what_each_state_allows() {
        use Activity::*;
        let rest = [("Play", false), ("Settings", false), ("Quit", true)];
        let idle = [("Start clicking", true), ("Record", true)];
        assert_eq!(shown(Idle), [&idle[..], &rest[..]].concat());
        let clicking = [("Stop clicking", true), ("Record", false)];
        for busy in [StartingIn(3), WaitingForRelease, Clicking] {
            assert_eq!(shown(busy), [&clicking[..], &rest[..]].concat(), "{busy:?}");
        }
        let recording = [("Start clicking", false), ("Stop recording", true)];
        for busy in [RecordingIn(2), Recording] {
            assert_eq!(
                shown(busy),
                [&recording[..], &rest[..]].concat(),
                "{busy:?}"
            );
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
            None,
            Some(Message::Quit),
        ];
        for activity in [Idle, Clicking, Recording, RecordingIn(1)] {
            assert_eq!(messages(activity), expected, "{activity:?}");
        }
    }

    #[test]
    fn the_icon_shows_clicking_and_recording() {
        use Activity::*;
        assert_eq!(icon_name(Clicking), "auto-clicker-clicking");
        assert_eq!(icon_name(Recording), "auto-clicker-recording");
        for waiting in [Idle, StartingIn(1), WaitingForRelease, RecordingIn(1)] {
            assert_eq!(icon_name(waiting), "auto-clicker-idle", "{waiting:?}");
        }
    }

    #[test]
    fn the_tray_shows_the_view_and_its_menu_sends_messages() {
        use ksni::Tray as _;
        let (send, mut received) = mpsc::unbounded_channel();
        let mut tray = Tray {
            view: view(Activity::Clicking),
            send,
        };
        assert_eq!(tray.id(), "auto-clicker");
        assert_eq!(tray.title(), "Auto clicker: clicking");
        assert_eq!(tray.icon_name(), "auto-clicker-clicking");
        // The source tree's icons in a development build, the theme's
        // otherwise.
        let path = tray.icon_theme_path();
        if cfg!(debug_assertions) {
            assert!(path.ends_with("/data/icons"), "{path}");
        } else {
            assert_eq!(path, "");
        }
        assert_eq!(tray.tool_tip().title, "Auto clicker: clicking");
        assert!(tray.watcher_offline(ksni::OfflineReason::No));
        let items = tray.menu();
        let shown: Vec<(String, bool, bool)> = items
            .iter()
            .map(|item| match item {
                MenuItem::Standard(item) => (item.label.clone(), item.enabled, item.visible),
                _ => ("-".to_owned(), false, true),
            })
            .collect();
        let expected = [
            ("Start clicking", false, false),
            ("Stop clicking", true, true),
            ("Record", false, true),
            ("Stop recording", true, false),
            ("Play", false, true),
            ("Settings", false, true),
            ("-", false, true),
            ("Quit", true, true),
        ];
        let expected = expected.map(|(label, on, shows)| (label.to_owned(), on, shows));
        assert_eq!(shown, expected);
        for index in [0, 1, 2, 3, 7] {
            if let MenuItem::Standard(item) = &items[index] {
                (item.activate)(&mut tray);
            }
        }
        tray.menu_about_to_show();
        let sent: Vec<Message> = std::iter::from_fn(|| received.try_recv().ok()).collect();
        assert_eq!(
            sent,
            [
                Message::Start,
                Message::Stop,
                Message::Record,
                Message::StopRecording,
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
