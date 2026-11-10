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

/// One menu item: its label, the message it sends (`None`: disabled), and
/// whether it shows.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    label: &'static str,
    sends: Option<Message>,
    visible: bool,
}

/// The menu for `view`. The items are always the same, in the same places,
/// each with one fixed message; only which of Start and Stop shows changes.
/// The host clicks an item by its place, so a click on a menu opened before
/// the state changed still sends what that item said.
fn entries(view: &View) -> [Entry; 6] {
    let idle = view.activity == Activity::Idle;
    let item = |label, sends, visible| Entry {
        label,
        sends,
        visible,
    };
    [
        item("Start clicking", Some(Message::Start), idle),
        item("Stop clicking", Some(Message::Stop), !idle),
        // Recording and playback come with M2, settings with M3.
        item("Record", None, true),
        item("Play", None, true),
        item("Settings", None, true),
        item("Quit", Some(Message::Quit), true),
    ]
}

/// The icon for `activity`, by name from the icon theme.
fn icon_name(activity: Activity) -> &'static str {
    match activity {
        Activity::Clicking => "auto-clicker-clicking",
        Activity::Idle | Activity::StartingIn(_) | Activity::WaitingForRelease => {
            "auto-clicker-idle"
        }
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
                let enabled = entry.sends.is_some();
                StandardItem {
                    label: entry.label.to_owned(),
                    enabled,
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
            notice: Some("a <b> & c".to_owned()),
            problems: Vec::new(),
        }
    }

    /// The visible items' labels and messages.
    fn shown(activity: Activity) -> Vec<(&'static str, Option<Message>)> {
        entries(&view(activity))
            .into_iter()
            .filter(|entry| entry.visible)
            .map(|entry| (entry.label, entry.sends))
            .collect()
    }

    #[test]
    fn the_menu_shows_start_when_idle_and_stop_otherwise() {
        let rest = [
            ("Record", None),
            ("Play", None),
            ("Settings", None),
            ("Quit", Some(Message::Quit)),
        ];
        let start = [("Start clicking", Some(Message::Start))];
        assert_eq!(shown(Activity::Idle), [&start[..], &rest[..]].concat());
        let stop = [("Stop clicking", Some(Message::Stop))];
        for busy in [
            Activity::StartingIn(3),
            Activity::WaitingForRelease,
            Activity::Clicking,
        ] {
            assert_eq!(shown(busy), [&stop[..], &rest[..]].concat(), "{busy:?}");
        }
    }

    #[test]
    fn every_item_keeps_its_place_and_message() {
        let messages = |activity| entries(&view(activity)).map(|entry| entry.sends);
        assert_eq!(messages(Activity::Idle), messages(Activity::Clicking));
    }

    #[test]
    fn only_clicking_shows_the_clicking_icon() {
        assert_eq!(icon_name(Activity::Clicking), "auto-clicker-clicking");
        assert_eq!(icon_name(Activity::StartingIn(1)), "auto-clicker-idle");
        assert_eq!(icon_name(Activity::WaitingForRelease), "auto-clicker-idle");
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
            ("Start clicking", true, false),
            ("Stop clicking", true, true),
            ("Record", false, true),
            ("Play", false, true),
            ("Settings", false, true),
            ("-", false, true),
            ("Quit", true, true),
        ];
        let expected = expected.map(|(label, on, shows)| (label.to_owned(), on, shows));
        assert_eq!(shown, expected);
        for index in [0, 1, 6] {
            if let MenuItem::Standard(item) = &items[index] {
                (item.activate)(&mut tray);
            }
        }
        assert_eq!(received.try_recv(), Ok(Message::Start));
        assert_eq!(received.try_recv(), Ok(Message::Stop));
        assert_eq!(received.try_recv(), Ok(Message::Quit));
    }

    #[test]
    fn tooltip_text_is_escaped() {
        let body = tooltip_body(&view(Activity::Idle));
        assert!(body.ends_with("<br/>a &lt;b&gt; &amp; c"), "{body}");
    }
}
