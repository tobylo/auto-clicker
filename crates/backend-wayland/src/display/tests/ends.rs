//! How the display's connection ends and is made again: by the view, its
//! own setup's deadline, a desktop that cannot show, and the next
//! countdown.

use super::*;

#[tokio::test]
async fn the_next_countdown_tries_a_lost_connection_again_and_idle_opens_nothing() {
    local(async {
        let mut harness = Harness::new().await;
        let layout = harness
            .wayland
            .watch(LayoutSignal::default(), LayoutSignal::default());
        let _layout = tokio::task::spawn_local(layout);
        harness.set(Activity::StartingIn(5));
        drop(harness.connected().await);
        // The loss is seen: a tick brings nothing back.
        harness.set(Activity::StartingIn(4));
        assert!(!harness.connects().await, "tried again on a tick");
        harness.set(Activity::Idle);
        // While nothing should show, a change of the screens opens nothing.
        harness.layout.send(vec![removed(B)]).await;
        timeout(BOUND, harness.layout.round_trip()).await.unwrap();
        assert!(!harness.connects().await, "connected while idle");
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
    })
    .await;
}

#[tokio::test]
async fn a_display_connection_that_cannot_show_is_closed_and_not_retried_on_a_tick() {
    // No layer-shell at all, or surfaces too old for the requests made.
    let offered = |old: bool| {
        let offers = [Offer::Compositor(3), Offer::Shm, Offer::LayerShell];
        offers.into_iter().filter(move |_| old)
    };
    for old in [false, true] {
        local(async {
            let harness = Harness::new().await;
            harness.set(Activity::StartingIn(5));
            let mut fake = timeout(BOUND, harness.display.accept()).await.unwrap();
            // Its outputs and what is offered: the client leaves once it
            // has heard them.
            let callback = timeout(BOUND, fake.until_sync()).await.unwrap();
            for offer in [Offer::Manager, Offer::Output(A), Offer::Output(B)] {
                fake.offer(offer).await;
            }
            for offer in offered(old) {
                fake.offer(offer).await;
            }
            fake.answer(callback).await;
            closes(&mut fake).await;
            harness.set(Activity::StartingIn(4));
            assert!(!harness.connects().await, "tried again on a tick");
        })
        .await;
    }
}

#[tokio::test]
async fn a_setup_never_answered_is_given_up_while_the_countdown_runs() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = timeout(BOUND, harness.display.accept()).await.unwrap();
        timeout(BOUND, fake.until_sync()).await.unwrap();
        let closed = timeout(BOUND, fake.closed()).await;
        assert_eq!(closed.ok(), Some(true), "the setup was waited on");
        harness.set(Activity::StartingIn(4));
        assert!(!harness.connects().await, "tried again on a tick");
    })
    .await;
}

#[tokio::test]
async fn the_controller_gone_closes_the_displays_connection() {
    local(async {
        let harness = Harness::new().await;
        harness.set(Activity::StartingIn(5));
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
        drop(harness.views);
        closes(&mut fake).await;
    })
    .await;
}

#[tokio::test]
async fn a_second_watcher_shows_nothing() {
    local(async {
        let harness = Harness::new().await;
        let second = harness.on_screen.watch(harness.views.subscribe());
        let _second = tokio::task::spawn_local(second);
        harness.set(Activity::StartingIn(5));
        let _fake = harness.connected().await;
        assert!(!harness.connects().await, "a second connection");
    })
    .await;
}

#[tokio::test]
async fn a_screen_change_that_leaves_the_layout_unknown_brings_a_lost_connection_back() {
    local(async {
        let mut harness = Harness::new().await;
        let layout = harness
            .wayland
            .watch(LayoutSignal::default(), LayoutSignal::default());
        let _layout = tokio::task::spawn_local(layout);
        // A screen too old to say when it changed: the layout is unknown
        // while it is there.
        harness.layout.offer(Offer::OldOutput(C, 1)).await;
        timeout(BOUND, harness.layout.round_trip()).await.unwrap();
        let unknown = timeout(BOUND, async {
            while harness.wayland.layout() != Err(NoScreens::NotWhole) {
                yield_now().await;
            }
        })
        .await;
        assert!(unknown.is_ok(), "{:?}", harness.wayland.layout());
        harness.set(Activity::StartingIn(5));
        drop(harness.connected().await);
        harness.set(Activity::StartingIn(4));
        assert!(!harness.connects().await, "tried again on a tick");
        // Another screen goes: the layout stays unknown, and the display
        // is tried again.
        harness.layout.send(vec![removed(B)]).await;
        timeout(BOUND, harness.layout.round_trip()).await.unwrap();
        let mut fake = harness.connected().await;
        answer(&mut fake, A).await;
    })
    .await;
}
