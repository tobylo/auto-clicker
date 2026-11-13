//! Why recordings keep movement only, as the view says it: each reason the
//! platform gives, a source that comes or goes, and a recording made
//! without positions (W01 brief, criterion 8).

use std::fs;

use super::recording::KEYBOARD;
use super::*;
use crate::message::NoPositions;
use crate::recording::Change;
use crate::recording::tests::positions::two_screens;

const W: u16 = 17;

/// A daemon whose platform has `layout`, unknown for `no_screens`, and
/// whose pointer is missing for `no_pointer`.
async fn daemon_lacking(
    layout: Option<Layout>,
    no_screens: Option<NoScreens>,
    no_pointer: Option<NoPointer>,
) -> Daemon {
    let shared = Shared::default();
    shared.with(|r| {
        r.layout = layout;
        r.no_screens = no_screens;
        r.no_pointer = no_pointer;
        r.recordings = Some(Arc::new(TempDir::new()));
    });
    daemon_with(config(Mode::Click), shared, true).await
}

impl Daemon {
    /// Why recordings keep movement only, as the view says it.
    fn no_positions(&self) -> Vec<NoPositions> {
        self.view()
            .problems
            .iter()
            .filter_map(|problem| match problem {
                Problem::NoPositions(why) => Some(*why),
                _ => None,
            })
            .collect()
    }

    /// Tells the controller what the layout watcher would after a change of
    /// the layout or the pointer, and lets it handle that.
    async fn source_changed(&self) {
        self.shared.with(|r| {
            for (_, changed) in &r.watches {
                changed.changed();
            }
        });
        self.settle().await;
    }
}

#[tokio::test(start_paused = true)]
async fn each_reason_reaches_the_view_as_its_reason() {
    local(async {
        let screens = [
            NoScreens::NoDisplay,
            NoScreens::CannotConnect,
            NoScreens::NoXdgOutput,
            NoScreens::Lost,
            NoScreens::ProtocolError,
            NoScreens::NotWhole,
        ];
        for why in screens {
            // Without screens, nothing else matters.
            let d = daemon_lacking(None, Some(why), Some(NoPointer::NoSource)).await;
            assert_eq!(d.no_positions(), [NoPositions::Screens(why)]);
        }
        for why in [NoPointer::NoSource, NoPointer::Unusable] {
            let d = daemon_lacking(Some(two_screens()), None, Some(why)).await;
            assert_eq!(d.no_positions(), [NoPositions::Pointer(why)]);
            assert_eq!(
                d.view().details().last().map(String::as_str),
                Some(format!("Recordings keep movement only: {why}").as_str())
            );
        }
        let whole = daemon_lacking(Some(two_screens()), None, None).await;
        assert_eq!(whole.no_positions(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn nothing_is_said_of_positions_where_no_recording_can_start() {
    local(async {
        let shared = Shared::default();
        shared.with(|r| r.desktop_reader = true);
        let d = daemon_with(config(Mode::Click), shared, true).await;
        assert!(d.view().no_recording.is_some());
        assert_eq!(d.no_positions(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_source_that_comes_or_goes_changes_the_reason_in_the_view() {
    local(async {
        let d = daemon_lacking(Some(two_screens()), None, None).await;
        assert_eq!(d.no_positions(), []);
        // KWin goes: nothing reports the pointer.
        d.shared.with(|r| r.no_pointer = Some(NoPointer::NoSource));
        d.source_changed().await;
        let gone = [NoPositions::Pointer(NoPointer::NoSource)];
        assert_eq!(d.no_positions(), gone);
        // A new KWin: the display connects again, the layout unknown
        // meanwhile.
        d.shared.with(|r| {
            r.layout = None;
            r.no_screens = Some(NoScreens::Lost);
        });
        d.source_changed().await;
        assert_eq!(d.no_positions(), [NoPositions::Screens(NoScreens::Lost)]);
        // It answers another listener: a new reason, the layout still
        // unknown.
        d.shared
            .with(|r| r.no_screens = Some(NoScreens::CannotConnect));
        d.source_changed().await;
        let other = [NoPositions::Screens(NoScreens::CannotConnect)];
        assert_eq!(d.no_positions(), other);
        // Connected again, and the new KWin counted.
        d.shared.with(|r| {
            r.layout = Some(two_screens());
            r.no_pointer = None;
        });
        d.source_changed().await;
        assert_eq!(d.no_positions(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn without_a_pointer_a_recording_keeps_movement_and_saves_as_version_1() {
    local(async {
        let d = daemon_lacking(Some(two_screens()), None, Some(NoPointer::NoSource)).await;
        d.record_hotkey().await;
        assert_eq!(d.activity(), Activity::Recording);
        // While recording, the tooltip still says why.
        assert_eq!(
            d.no_positions(),
            [NoPositions::Pointer(NoPointer::NoSource)]
        );
        sleep(Duration::from_millis(100)).await;
        let now = d.shared.clock();
        d.shared.feed(|recorder| recorder.motion(now, 3, -2));
        d.key(KEYBOARD, W, true);
        d.key(KEYBOARD, W, false);
        d.record_hotkey().await;
        let saved = d.saved();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].layout(), None);
        assert_eq!(saved[0].events()[0].change, Change::Move { dx: 3, dy: -2 });
        let dir = d.shared.with(|r| r.recordings.clone()).unwrap();
        let text = fs::read_to_string(dir.path().join(&dir.entries()[0])).unwrap();
        assert!(text.starts_with("{\"version\":1,"), "{text}");
    })
    .await;
}
