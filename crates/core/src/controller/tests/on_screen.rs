//! The on-screen display as the controller sees it: one more reader of the
//! view, and why it cannot show, in the tooltip while it is switched on
//! (O01's O1 and O3).

use super::*;
use crate::config::OnScreenSettings;

/// A daemon whose platform's display cannot show for `why`, the
/// countdown switched on or off.
async fn daemon_lacking(why: Option<NoOnScreen>, countdown: bool) -> Daemon {
    let shared = Shared::default();
    shared.with(|r| r.no_on_screen = why);
    let config = Config {
        on_screen: OnScreenSettings { countdown },
        ..config(Mode::Click)
    };
    daemon_with(config, shared, true).await
}

impl Daemon {
    /// Why the countdown cannot show, as the view says it.
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

#[tokio::test(start_paused = true)]
async fn why_the_countdown_cannot_show_is_listed_while_it_is_switched_on() {
    local(async {
        for why in [
            NoOnScreen::NoDisplay,
            NoOnScreen::CannotConnect,
            NoOnScreen::NoXdgOutput,
            NoOnScreen::NoLayerShell,
        ] {
            let d = daemon_lacking(Some(why), true).await;
            assert_eq!(d.no_on_screen(), [why]);
            // Recording does not depend on it.
            let able = daemon_lacking(None, true).await;
            assert_eq!(d.view().no_recording, able.view().no_recording);
            // Switched off, nothing would show anyway.
            let off = daemon_lacking(Some(why), false).await;
            assert!(off.no_on_screen().is_empty(), "{why:?}");
            assert!(!off.view().on_screen.countdown);
        }
        let d = daemon_lacking(None, true).await;
        assert!(d.no_on_screen().is_empty());
        assert!(d.view().on_screen.countdown);
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
