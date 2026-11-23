//! The on-screen display as the controller sees it: one more reader of the
//! view, and why it cannot show: in the tooltip while any of its parts is
//! switched on, and to the settings window whatever they say (O01's O1 and
//! O3).

use super::settings::setup_with;
use super::*;
use crate::config::OnScreenSettings;

const EVERY_WHY: [NoOnScreen; 4] = [
    NoOnScreen::NoDisplay,
    NoOnScreen::CannotConnect,
    NoOnScreen::NoXdgOutput,
    NoOnScreen::NoLayerShell,
];

/// Nothing switched on.
const ALL_OFF: OnScreenSettings = OnScreenSettings {
    countdown: false,
    clicking: false,
    recording: false,
    playback: false,
    corner: crate::config::Corner::TopLeft,
};

/// A daemon whose platform's display cannot show for `why`, its parts
/// switched on as `on_screen` says.
async fn daemon_lacking(why: Option<NoOnScreen>, on_screen: OnScreenSettings) -> Daemon {
    let shared = Shared::default();
    shared.with(|r| r.no_on_screen = why);
    let config = Config {
        on_screen,
        ..config(Mode::Click)
    };
    daemon_with(config, shared, true).await
}

impl Daemon {
    /// Why the display cannot show, as the view says it.
    fn no_on_screen(&self) -> Vec<NoOnScreen> {
        self.view()
            .problems
            .iter()
            .filter_map(|problem| match problem {
                Problem::NoOnScreen(why) => Some(*why),
                _ => None,
            })
            .collect()
    }
}

/// Each part alone switched on.
fn each_alone() -> [(&'static str, OnScreenSettings); 4] {
    [
        (
            "countdown",
            OnScreenSettings {
                countdown: true,
                ..ALL_OFF
            },
        ),
        (
            "clicking",
            OnScreenSettings {
                clicking: true,
                ..ALL_OFF
            },
        ),
        (
            "recording",
            OnScreenSettings {
                recording: true,
                ..ALL_OFF
            },
        ),
        (
            "playback",
            OnScreenSettings {
                playback: true,
                ..ALL_OFF
            },
        ),
    ]
}

#[tokio::test(start_paused = true)]
async fn why_the_display_cannot_show_is_listed_while_any_part_is_switched_on() {
    local(async {
        for why in EVERY_WHY {
            for (part, on_screen) in each_alone() {
                let d = daemon_lacking(Some(why), on_screen).await;
                assert_eq!(d.no_on_screen(), [why], "{part}");
            }
            let d = daemon_lacking(Some(why), OnScreenSettings::default()).await;
            assert_eq!(d.no_on_screen(), [why]);
            // Recording does not depend on it.
            let able = daemon_lacking(None, OnScreenSettings::default()).await;
            assert_eq!(d.view().no_recording, able.view().no_recording);
            // All switched off, nothing would show anyway.
            let off = daemon_lacking(Some(why), ALL_OFF).await;
            assert!(off.no_on_screen().is_empty(), "{why:?}");
            assert_eq!(off.view().on_screen, ALL_OFF);
        }
        let d = daemon_lacking(None, OnScreenSettings::default()).await;
        assert!(d.no_on_screen().is_empty());
        assert_eq!(d.view().on_screen, OnScreenSettings::default());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_window_hears_why_the_display_cannot_show_whatever_the_switches_say() {
    local(async {
        let off = "version = 1\n[on_screen]\ncountdown = false\nclicking = false\n\
                   recording = false\nplayback = false\n";
        for why in EVERY_WHY.map(Some).into_iter().chain([None]) {
            for text in [None, Some(off)] {
                let shared = Shared::default();
                shared.with(|r| r.no_on_screen = why);
                let s = setup_with(text, shared).await;
                assert_eq!(s.window.view().no_on_screen, why, "{text:?}");
            }
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_layout_watcher_reads_the_views_the_controller_publishes() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        let views = d.shared.with(|r| r.views.clone());
        assert_eq!(views.len(), 1);
        let mut views = views.into_iter().next().unwrap();
        d.send(Message::Start).await;
        assert!(views.has_changed().unwrap());
        assert_eq!(views.borrow_and_update().activity, Activity::StartingIn(5));
        d.send(Message::Stop).await;
        d.settle().await;
        assert_eq!(views.borrow_and_update().activity, Activity::Idle);
    })
    .await;
}
