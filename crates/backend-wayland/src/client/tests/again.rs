//! Connecting again: to the listener found at start, each time a new KWin
//! of the display comes, and never otherwise.

use super::*;

#[tokio::test]
async fn a_display_that_does_not_answer_in_time_keeps_its_listener() {
    let (link, _fake) = Fake::pair();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
    let path = PathBuf::from("/nowhere");
    let wayland = timeout(BOUND, Wayland::over(link, deadline, path))
        .await
        .unwrap();
    assert_eq!(wayland.layout().err(), Some(NoScreens::CannotConnect));
    assert_eq!(wayland.layout().ok(), None);
    assert_eq!(
        wayland.compositor().map(NonZeroU32::get),
        Some(std::process::id())
    );
}

/// A watched connection on two outputs to a display at its own path,
/// listened on by this process.
async fn watched_display(local: &LocalSet, display: &Display) -> (Watched, Fake) {
    let (link, mut fake) = Fake::pair();
    let at = At {
        path: Some(display.path.clone()),
        compositor: NonZeroU32::new(std::process::id()),
    };
    let watched = watched_at(local, &mut fake, link, at).await;
    (watched, fake)
}

#[tokio::test]
async fn a_new_kwin_connects_again_and_the_layout_comes_back() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (watched, fake) = watched_display(&local, &display).await;
            drop(fake);
            timeout(BOUND, until_missing(&watched.wayland))
                .await
                .unwrap();
            assert_eq!(watched.wayland.layout().ok(), None);
            watched.again.changed();
            let _fake = timeout(BOUND, async {
                let mut fake = display.accept().await;
                fake.start_with_two().await;
                until_known(&watched.wayland).await;
                fake
            })
            .await
            .unwrap();
            settle().await;
            assert_eq!(watched.wayland.layout().ok(), layout(&[LEFT, RIGHT]));
            assert_eq!(watched.wayland.layout().err(), None);
            assert_eq!(watched.fired.get(), 2, "unknown, then known again");
        })
        .await;
}

#[tokio::test]
async fn a_new_kwin_replaces_a_connection_still_open() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (watched, mut old) = watched_display(&local, &display).await;
            watched.again.changed();
            let _fake = timeout(BOUND, async {
                let mut fake = display.accept().await;
                assert!(old.closed().await, "the old connection stayed open");
                assert_eq!(watched.wayland.layout().ok(), None);
                fake.start_with_two().await;
                until_known(&watched.wayland).await;
                fake
            })
            .await
            .unwrap();
            settle().await;
            assert_eq!(watched.wayland.layout().ok(), layout(&[LEFT, RIGHT]));
        })
        .await;
}

#[tokio::test]
async fn a_newer_kwin_during_a_reconnect_starts_it_over() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (watched, fake) = watched_display(&local, &display).await;
            drop(fake);
            watched.again.changed();
            let _second = timeout(BOUND, async {
                // The first new connection is never answered; a newer KWin
                // comes meanwhile.
                let mut first = display.accept().await;
                watched.again.changed();
                let mut second = display.accept().await;
                assert!(first.closed().await, "the first try stayed open");
                second.start_with_two().await;
                until_known(&watched.wayland).await;
                second
            })
            .await
            .unwrap();
            settle().await;
            assert_eq!(watched.wayland.layout().err(), None);
        })
        .await;
}

#[tokio::test]
async fn a_display_another_process_listens_on_now_gives_no_layout() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (link, mut fake) = Fake::pair();
            // At start, the display's listener was another process.
            let at = At {
                path: Some(display.path.clone()),
                compositor: NonZeroU32::new(1),
            };
            let watched = watched_at(&local, &mut fake, link, at).await;
            drop(fake);
            watched.again.changed();
            timeout(BOUND, async {
                let mut fake = display.accept().await;
                assert!(fake.closed().await, "the connection stayed open");
                while watched.wayland.layout().err() != Some(NoScreens::CannotConnect) {
                    settle().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(watched.wayland.layout().ok(), None);
            assert!(!watched.watch.is_finished());
        })
        .await;
}

#[tokio::test]
async fn without_a_new_kwin_nothing_connects_again() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (watched, fake) = watched_display(&local, &display).await;
            drop(fake);
            timeout(BOUND, until_missing(&watched.wayland))
                .await
                .unwrap();
            let quiet = timeout(Duration::from_millis(200), display.accept()).await;
            assert!(quiet.is_err(), "connected again unasked");
            assert_eq!(watched.wayland.layout().err(), Some(NoScreens::Lost));
        })
        .await;
}

#[tokio::test]
async fn a_second_watch_watches_nothing() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (watched, _fake) = watched_display(&local, &display).await;
            // A signal of its own: the first watch never takes its fire.
            let again = LayoutSignal::default();
            local.spawn_local(
                watched
                    .wayland
                    .watch(LayoutSignal::default(), again.clone()),
            );
            again.changed();
            let quiet = timeout(Duration::from_millis(200), display.accept()).await;
            assert!(quiet.is_err(), "a second watch connected");
        })
        .await;
}

/// Connects again after a new KWin, and reads the layout back.
async fn comes_back(watched: &Watched, display: &Display) {
    watched.again.changed();
    let _fake = timeout(BOUND, async {
        let mut fake = display.accept().await;
        fake.start_with_two().await;
        until_known(&watched.wayland).await;
        fake
    })
    .await
    .unwrap();
    assert_eq!(watched.wayland.layout().ok(), layout(&[LEFT, RIGHT]));
    assert_eq!(watched.wayland.layout().err(), None);
}

#[tokio::test]
async fn a_display_that_failed_at_start_connects_at_the_next_kwin() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let wayland = Wayland::of(
                NonZeroU32::new(std::process::id()),
                Some(display.path.clone()),
                Settled::default(),
                Err(NoScreens::CannotConnect),
            );
            let again = LayoutSignal::default();
            let watch = local.spawn_local(wayland.watch(LayoutSignal::default(), again.clone()));
            let watched = Watched {
                wayland,
                fired: Rc::default(),
                again,
                watch,
            };
            comes_back(&watched, &display).await;
        })
        .await;
}

#[tokio::test]
async fn a_listener_unknown_at_start_takes_whoever_listens_now() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (link, mut fake) = Fake::pair();
            let at = At {
                path: Some(display.path.clone()),
                compositor: None,
            };
            let watched = watched_at(&local, &mut fake, link, at).await;
            drop(fake);
            comes_back(&watched, &display).await;
        })
        .await;
}

#[tokio::test]
async fn a_new_kwin_makes_the_layout_unknown_in_its_poll_whatever_the_budget() {
    use std::future::{Future, poll_fn};
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    let (link, mut fake) = Fake::pair();
    let (started, ()) = timeout(BOUND, async {
        tokio::join!(Link::start(link), fake.start_with_two())
    })
    .await
    .unwrap();
    let (link, first) = started.unwrap();
    let wayland = Wayland::of(
        NonZeroU32::new(std::process::id()),
        Some(PathBuf::from("/nowhere")),
        first,
        Ok(link),
    );
    let (changed, again) = (LayoutSignal::default(), LayoutSignal::default());
    let mut watch = wayland.watch(changed.clone(), again.clone());
    // Polled here, in the test's own task, so the budget it spends counts.
    poll_fn(|cx| {
        assert!(watch.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    again.changed();
    poll_fn(|cx| {
        spend_budget(cx);
        assert!(watch.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let mut cx = Context::from_waker(Waker::noop());
    assert!(
        pin!(changed.fired()).poll(&mut cx).is_ready(),
        "the restart waited for a budget"
    );
    assert_eq!(
        (wayland.layout().ok(), wayland.layout().err()),
        (None, Some(NoScreens::Lost))
    );
}

/// Spends the task's whole budget.
fn spend_budget(cx: &mut std::task::Context<'_>) {
    use std::future::Future;

    use tokio::task::coop::consume_budget;

    while std::pin::pin!(consume_budget()).poll(cx).is_ready() {}
}
