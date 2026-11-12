//! KWin counted only when it runs the daemon's Wayland display (brief W1).

use super::*;
use crate::display::Parent;

/// `kwin_wayland_wrapper`, listening on the daemon's display.
const WRAPPER: u32 = 1487;
/// `kwin_wayland` under it, owning `KWIN`.
const OURS: Process = Process {
    pid: 1491,
    parent: Parent::Of(WRAPPER),
};
/// A KWin of another session.
const OTHER: Process = Process {
    pid: 2210,
    parent: Parent::Of(2201),
};

fn display() -> Option<NonZeroU32> {
    NonZeroU32::new(WRAPPER)
}

/// A harness on the wrapper's display whose KWin is `process`.
fn kwin_as(process: Process) -> (Harness, Inbox) {
    let (h, inbox) = Harness::on_display(true, &[], display());
    h.kwin.with(|s| s.processes.push((KWIN, process)));
    (h, inbox)
}

#[tokio::test(start_paused = true)]
async fn the_kwin_of_the_display_gives_its_layout() {
    local(async {
        let (h, inbox) = kwin_as(OURS);
        h.start(inbox).await;
        let swept = [Unload(LAYOUT), Unload(POSITION)];
        assert_eq!(h.calls(), [swept.as_slice(), &LOAD_LAYOUT].concat());
        assert_eq!(h.layout(), two_screens());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_kwin_of_another_display_is_ignored_and_called_for_nothing() {
    local(async {
        let (h, inbox) = kwin_as(OTHER);
        h.start(inbox).await;
        h.record().await;
        after(HOUR).await;
        assert_eq!(h.calls(), []);
        assert_eq!(h.layout(), None);
        assert!(h.stop().await.is_empty(), "a position from the other KWin");
        assert_eq!(h.session.trust.lock().unwrap().owner, None);
        // Nor is anything unloaded from it at exit.
        h.session.exit().await;
        assert_eq!(h.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_exit_that_cannot_tell_whose_kwin_it_is_unloads_nothing() {
    local(async {
        let (h, _inbox) = kwin_as(OURS);
        h.kwin.with(|s| s.failing_lookups = 1);
        h.session.exit().await;
        assert_eq!(h.calls(), []);
        // Told, it unloads what a crashed daemon may have left.
        h.session.exit().await;
        assert_eq!(h.calls(), [Unload(LAYOUT), Unload(POSITION)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn each_new_owner_is_checked_before_anything_is_loaded() {
    local(async {
        let (h, inbox) = kwin_as(OURS);
        h.start(inbox).await;
        h.calls();
        // KWin restarts under the same wrapper: counted, and loaded again.
        h.kwin.with(|s| {
            s.processes.push((
                ":1.30",
                Process {
                    pid: 1700,
                    parent: Parent::Of(WRAPPER),
                },
            ));
        });
        h.new_owner(Some(":1.30")).await;
        assert_eq!(h.calls(), LOAD_LAYOUT);
        assert_eq!(h.layout(), two_screens());
        // Another session's KWin takes the name: the layout goes, and
        // nothing is loaded into it.
        h.kwin.with(|s| s.processes.push((":1.40", OTHER)));
        h.new_owner(Some(":1.40")).await;
        assert_eq!(h.calls(), []);
        assert_eq!(h.layout(), None);
        assert!(h.told().await);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_lookup_that_fails_is_tried_again_and_then_trusted() {
    local(async {
        let (h, inbox) = kwin_as(OTHER);
        h.kwin.with(|s| s.failing_lookups = 1);
        h.start(inbox).await;
        assert_eq!(h.calls(), [], "loaded before KWin was told");
        after(BACK_OFF[0]).await;
        // The second lookup answered: another display's.
        assert_eq!(h.kwin.with(|s| s.lookups), 2);
        assert_eq!(h.calls(), []);

        // A KWin that never answers is trusted once the tries are spent.
        let (h, inbox) = kwin_as(OURS);
        h.kwin.with(|s| s.failing_lookups = usize::MAX);
        h.start(inbox).await;
        for wait in BACK_OFF {
            assert_eq!(h.layout(), None);
            after(wait).await;
        }
        assert_eq!(h.kwin.with(|s| s.lookups), 1 + BACK_OFF.len());
        assert_eq!(h.layout(), two_screens());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn without_a_display_to_check_against_kwin_counts_unasked() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        assert_eq!(h.layout(), two_screens());
        assert_eq!(h.kwin.with(|s| s.lookups), 0);
    })
    .await;
}

/// KWin restarted under the wrapper, as `:1.30`.
const RESTARTED: Process = Process {
    pid: 1700,
    parent: Parent::Of(WRAPPER),
};

#[tokio::test(start_paused = true)]
async fn a_new_owner_drops_the_check_of_the_one_before() {
    local(async {
        let (h, inbox) = kwin_as(OURS);
        h.start(inbox).await;
        h.calls();
        // An owner whose lookup fails, so its check backs off, then a
        // newer one: the newer is checked and loaded at once.
        h.new_owner(Some(":1.20")).await;
        h.kwin.with(|s| s.processes.push((":1.30", RESTARTED)));
        h.new_owner(Some(":1.30")).await;
        assert_eq!(h.calls(), LOAD_LAYOUT);
        assert_eq!(h.layout(), two_screens());
        // The dropped check never comes back to count `:1.20`.
        after(HOUR).await;
        assert_eq!(h.calls(), []);
        assert_eq!(h.layout(), two_screens());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_new_owner_during_the_first_check_is_checked_at_once() {
    local(async {
        // The first owner's lookup fails: its check backs off.
        let (h, inbox) = Harness::on_display(true, &[], display());
        h.start(inbox).await;
        h.kwin.with(|s| s.processes.push((":1.30", RESTARTED)));
        h.new_owner(Some(":1.30")).await;
        assert_eq!(h.calls(), LOAD_LAYOUT);
        assert_eq!(h.layout(), two_screens());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_kwin_whose_parent_cannot_be_read_is_tried_again() {
    local(async {
        let unread = Process {
            parent: Parent::Unknown,
            ..OURS
        };
        let (h, inbox) = kwin_as(unread);
        h.start(inbox).await;
        assert_eq!(h.calls(), [], "loaded before KWin was told");
        // Read on the next try: it is the display's.
        h.kwin.with(|s| s.processes = vec![(KWIN, OURS)]);
        after(BACK_OFF[0]).await;
        assert_eq!(h.kwin.with(|s| s.lookups), 2);
        assert_eq!(h.layout(), two_screens());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_new_owner_reads_as_none_while_it_is_checked() {
    local(async {
        let (h, inbox) = kwin_as(OURS);
        h.start(inbox).await;
        h.calls();
        assert!(h.told().await);
        // Its lookup fails, so its check backs off: the old KWin's layout
        // is gone meanwhile.
        h.new_owner(Some(":1.40")).await;
        assert_eq!(h.layout(), None);
        assert!(h.told().await);
        assert_eq!(h.calls(), []);
    })
    .await;
}
