//! The tray icon: shows the controller's view and sends it what the menu
//! asks for. It never changes the mode itself.

use auto_clicker_core::controller::{Activity, Message, View};
use auto_clicker_core::digits;
use auto_clicker_core::locale::Language;
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use ksni::{MenuItem, ToolTip, TrayMethods};
use tokio::sync::{mpsc, watch};

use crate::tray_icon::{self, Image, Picture};
use crate::tray_menu::{Choice, Entry, entries};
use crate::tray_text;

/// The sizes Plasma picks a countdown's pixmap from: the default panel's,
/// and larger panels' or scaled screens'.
const SIZES: [u16; 3] = [22, 32, 48];

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

/// The themed icon for `activity`, by name. A countdown shows the idle
/// picture here; the tray draws its seconds instead (see [`tray_icon`]).
fn icon_name(activity: Activity) -> &'static str {
    match Picture::of(activity) {
        Picture::Clicking => "auto-clicker-clicking",
        Picture::Recording => "auto-clicker-recording",
        Picture::Idle => "auto-clicker-idle",
    }
}

/// The icon's pixmaps for `activity`: one per size during a countdown, none
/// otherwise.
fn pixmaps(activity: Activity) -> Vec<ksni::Icon> {
    SIZES
        .iter()
        .filter_map(|&size| tray_icon::countdown(activity, size))
        .map(pixmap)
        .collect()
}

/// `image` as Plasma wants it: ARGB32 in network byte order.
fn pixmap(image: Image) -> ksni::Icon {
    let data = image
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| [a, r, g, b])
        .collect();
    ksni::Icon {
        width: i32::from(image.side),
        height: i32::from(image.side),
        data,
    }
}

/// Tooltip text is markup on Plasma; config errors and device errors carry
/// text from outside, which must show as written.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn tooltip_body(view: &View, language: Language) -> String {
    tray_text::details(view, language)
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

    /// No name during a countdown, so Plasma shows the pixmaps instead.
    fn icon_name(&self) -> String {
        if digits::countdown(self.view.activity).is_some() {
            String::new()
        } else {
            icon_name(self.view.activity).to_owned()
        }
    }

    /// The seconds left during a countdown; nothing otherwise.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        pixmaps(self.view.activity)
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
            // The themed picture, even during a countdown, which the
            // title already spells out.
            icon_name: icon_name(self.view.activity).to_owned(),
            icon_pixmap: Vec::new(),
            title: escape(&tray_text::title(&self.view, self.language)),
            description: tooltip_body(&self.view, self.language),
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

    use auto_clicker_core::controller::{LISTED_RECORDINGS, Listed, NoRecording, Route, Routes};
    use auto_clicker_core::message::{Hindrance, Notice, Quoted, Start};

    use super::*;
    use crate::tray_menu::tests::{file, labelled, view, view_listing};

    /// The notice every test view shows, escaped.
    const FINISHED: &str = "Finished playing a &lt;b&gt; &amp; c (2026-11-09 18:00:00)";

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

    /// What the tray shows as its icon: the name, the pixmaps' sizes, and
    /// the tooltip's icon name.
    fn shown_icon(tray: &Tray) -> (String, Vec<i32>, String) {
        use ksni::Tray as _;
        let sizes = tray.icon_pixmap().iter().map(|icon| icon.width).collect();
        (tray.icon_name(), sizes, tray.tool_tip().icon_name)
    }

    #[test]
    fn a_countdown_shows_on_the_icon_and_the_themed_icon_comes_back() {
        use Activity::*;
        let (send, _received) = mpsc::unbounded_channel();
        let (settings, _opened) = mpsc::unbounded_channel();
        let mut tray = Tray::new(view(Idle), send, settings, Language::English);
        let themed = |name: &str| (name.to_owned(), Vec::new(), name.to_owned());
        let counting = (
            String::new(),
            vec![22, 32, 48],
            "auto-clicker-idle".to_owned(),
        );
        // Each countdown, then what ends it: cancelled, or the run starting.
        for (countdown, then) in [
            (StartingIn(5), Idle),
            (StartingIn(5), Clicking),
            (RecordingIn(5), Idle),
            (RecordingIn(5), Recording),
            (PlayingIn(5), Idle),
            (PlayingIn(5), Playing),
        ] {
            tray.show(view(countdown));
            assert_eq!(shown_icon(&tray), counting, "{countdown:?}");
            tray.show(view(then));
            assert_eq!(
                shown_icon(&tray),
                themed(icon_name(then)),
                "{countdown:?} then {then:?}"
            );
        }
        for other in [
            Idle,
            WaitingForRelease,
            Opening,
            Clicking,
            Recording,
            PlayWaitingForRelease,
            PlayOpening,
            Playing,
        ] {
            tray.show(view(other));
            assert_eq!(shown_icon(&tray), themed(icon_name(other)), "{other:?}");
        }
    }

    #[test]
    fn each_second_of_a_countdown_draws_a_new_pixmap() {
        use ksni::Tray as _;
        let (send, _received) = mpsc::unbounded_channel();
        let (settings, _opened) = mpsc::unbounded_channel();
        let mut tray = Tray::new(
            view(Activity::StartingIn(3)),
            send,
            settings,
            Language::English,
        );
        let three = tray.icon_pixmap();
        tray.show(view(Activity::StartingIn(2)));
        let two = tray.icon_pixmap();
        assert!(three.iter().zip(&two).all(|(a, b)| a.data != b.data));
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
    fn the_tooltip_gives_the_reason_and_the_fix_escaped_like_every_line() {
        let shown = View {
            no_recording: Some(NoRecording::DesktopHotkeys),
            notice: Some(Notice::Refused {
                start: Start::Recording,
                why: Hindrance::Unlisted(Quoted::new("<why>")),
            }),
            ..view(Activity::Idle)
        };
        let lines: Vec<String> = tooltip_body(&shown, Language::English)
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
            Some("Not recording: the recordings cannot be listed: &lt;why&gt;")
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
            let body = tooltip_body(&shown, Language::English);
            let lines: Vec<&str> = body.split("<br/>").collect();
            // After the hotkeys, before the notice, which stays escaped.
            assert_eq!(lines[4], line, "{body}");
            assert_eq!(lines[5], FINISHED, "{body}");
        }
    }

    #[test]
    fn tooltip_text_is_escaped() {
        let body = tooltip_body(&view(Activity::Idle), Language::English);
        assert!(body.ends_with(&format!("<br/>{FINISHED}")), "{body}");
    }
}
