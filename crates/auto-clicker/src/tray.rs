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

/// One menu item: its label, and the message it sends (`None`: disabled).
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    label: &'static str,
    sends: Option<Message>,
}

/// The menu for `view`. Each item says what it asks for, so a menu opened
/// before the state changed can never flip it the wrong way.
fn entries(view: &View) -> [Entry; 5] {
    let toggle = match view.activity {
        Activity::Idle => Entry {
            label: "Start clicking",
            sends: Some(Message::Start),
        },
        Activity::StartingIn(_) | Activity::WaitingForRelease | Activity::Clicking => Entry {
            label: "Stop clicking",
            sends: Some(Message::Stop),
        },
    };
    let disabled = |label| Entry { label, sends: None };
    [
        toggle,
        // Recording and playback come with M2, settings with M3.
        disabled("Record"),
        disabled("Play"),
        disabled("Settings"),
        Entry {
            label: "Quit",
            sends: Some(Message::Quit),
        },
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

    #[test]
    fn the_menu_starts_when_idle_and_stops_otherwise() {
        let first = |activity| entries(&view(activity))[0].sends.clone();
        assert_eq!(first(Activity::Idle), Some(Message::Start));
        for busy in [
            Activity::StartingIn(3),
            Activity::WaitingForRelease,
            Activity::Clicking,
        ] {
            assert_eq!(first(busy), Some(Message::Stop), "{busy:?}");
        }
    }

    #[test]
    fn record_play_and_settings_are_disabled_and_quit_quits() {
        let menu = entries(&view(Activity::Idle));
        let labels: Vec<_> = menu.iter().map(|e| (e.label, e.sends.clone())).collect();
        assert_eq!(
            labels[1..],
            [
                ("Record", None),
                ("Play", None),
                ("Settings", None),
                ("Quit", Some(Message::Quit)),
            ]
        );
    }

    #[test]
    fn only_clicking_shows_the_clicking_icon() {
        assert_eq!(icon_name(Activity::Clicking), "auto-clicker-clicking");
        assert_eq!(icon_name(Activity::StartingIn(1)), "auto-clicker-idle");
        assert_eq!(icon_name(Activity::WaitingForRelease), "auto-clicker-idle");
    }

    #[test]
    fn tooltip_text_is_escaped() {
        let body = tooltip_body(&view(Activity::Idle));
        assert!(body.ends_with("<br/>a &lt;b&gt; &amp; c"), "{body}");
    }
}
