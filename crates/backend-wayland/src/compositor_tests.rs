//! Against the running compositor: run by hand on a Wayland session with
//! `cargo test --locked -p auto-clicker-backend-wayland -- --ignored`.

use super::*;

#[tokio::test]
#[ignore = "needs a running Wayland compositor that offers xdg-output"]
async fn the_running_compositor_gives_a_layout_and_its_listener() {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    let wayland = Wayland::connect(deadline).await;
    let layout = wayland.layout().expect("no whole layout");
    eprintln!("layout: {layout:?}; listener: {:?}", wayland.compositor());
    assert!(wayland.compositor().is_some());
    assert!(!layout.outputs().is_empty());
}

#[tokio::test]
#[ignore = "needs a running Wayland compositor and a screen change within 60 s"]
async fn the_running_compositor_reports_a_screen_change() {
    use std::time::Duration;

    use tokio::time::timeout;

    let wayland = Wayland::connect(tokio::time::Instant::now() + Duration::from_secs(2)).await;
    let before = wayland.layout();
    eprintln!("before: {before:?}; change a screen's scale or place now");
    let changed = LayoutSignal::default();
    let watch = wayland.watch(changed.clone(), LayoutSignal::default());
    tokio::select! {
        () = watch => panic!("the connection ended"),
        fired = timeout(Duration::from_secs(60), changed.fired()) => {
            fired.expect("no change within 60 s");
        }
    }
    eprintln!("after: {:?}", wayland.layout());
    assert_ne!(wayland.layout(), before);
}

#[tokio::test]
#[ignore = "needs a running Wayland compositor with cursor sessions (niri 26.04 or later from main), and the pointer moved for 10 s"]
async fn the_running_compositor_gives_cursor_positions_while_recording() {
    use std::time::Duration;

    use auto_clicker_core::hotkeys::Bindings;
    use auto_clicker_core::recording::{Change, Created, Ending, Recorder, RecordingSlot};
    use tokio::task::LocalSet;

    /// Any clock does, the recorder's and the positions' being the same.
    fn clock() -> Duration {
        static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        EPOCH.get_or_init(std::time::Instant::now).elapsed()
    }

    LocalSet::new()
        .run_until(async {
            let wayland =
                Wayland::connect(tokio::time::Instant::now() + Duration::from_secs(2)).await;
            let layout = wayland.layout().expect("no whole layout");
            let cursors = Cursors::of(&wayland, clock);
            assert_eq!(cursors.found(), Some(Ok(())), "no cursor sessions offered");
            let slot = RecordingSlot::new();
            let watch = cursors.watch(slot.feed(), Box::new(|| true), LayoutSignal::default());
            tokio::task::spawn_local(watch);
            let created = Created::parse("2026-11-16T12:00:00.000+01:00").unwrap();
            let recorder = Recorder::new(Bindings::default(), clock(), created, None);
            slot.start(recorder.with_layout(layout.clone()));
            eprintln!("layout: {layout:?}; move the pointer for 10 s");
            tokio::time::sleep(Duration::from_secs(10)).await;
            let mut recorder = slot.take().unwrap();
            recorder.wheel(clock(), 1);
            let recording = recorder.finish(Ending::At(clock())).unwrap().unwrap();
            let positions: Vec<_> = recording
                .events()
                .iter()
                .filter_map(|event| match event.change {
                    Change::Position(point) => Some((event.at, point)),
                    _ => None,
                })
                .collect();
            eprintln!(
                "{} positions; first {:?}, last {:?}",
                positions.len(),
                positions.first(),
                positions.last()
            );
            assert_eq!(cursors.found(), Some(Ok(())));
            assert!(!positions.is_empty());
        })
        .await;
}

/// The display's view while `activity` runs, the badges in `corner`.
fn on_screen_view(
    activity: auto_clicker_core::controller::Activity,
    corner: auto_clicker_core::config::Corner,
) -> auto_clicker_core::controller::View {
    use auto_clicker_core::config::{Config, OnScreenSettings};
    use auto_clicker_core::controller::{Hotkey, Routes, View};

    let config = Config::default();
    View {
        activity,
        routes: Routes::default(),
        settings: config.clicker,
        hotkey: Hotkey::Bound(config.hotkeys.toggle_clicking),
        record_hotkey: Hotkey::Bound(config.hotkeys.toggle_recording),
        no_recording: None,
        play_hotkey: Hotkey::Bound(config.hotkeys.toggle_playback),
        playback: config.playback,
        on_screen: OnScreenSettings {
            corner,
            ..config.on_screen
        },
        recordings: std::sync::Arc::from([]),
        current: None,
        playing: None,
        notice: None,
        problems: Vec::new(),
    }
}

/// Waits up to 2 s until each of `screens` screens shows `text`.
async fn every_screen_shows(on_screen: &OnScreen, screens: usize, text: &str) {
    use crate::client::Showing;

    let all_drawn = || {
        let now = on_screen.screens();
        now.len() == screens && now.values().all(|s| *s == Showing::Drawn(text.to_owned()))
    };
    let drawn = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !all_drawn() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(drawn.is_ok(), "{text}: {:?}", on_screen.screens());
    eprintln!("{text} on {screens} screens");
}

#[tokio::test]
#[ignore = "needs a running Wayland compositor with layer-shell; shows a countdown, then each badge in each corner, on every screen for about 20 seconds"]
async fn the_running_compositor_shows_the_countdown_and_the_badges_on_every_screen_and_removes_them()
 {
    use std::time::Duration;

    use auto_clicker_core::config::Corner;
    use auto_clicker_core::controller::Activity;
    use tokio::task::{LocalSet, yield_now};
    use tokio::time::{sleep, timeout};

    let view = |activity| on_screen_view(activity, Corner::TopLeft);
    LocalSet::new()
        .run_until(async {
            let wayland =
                Wayland::connect(tokio::time::Instant::now() + Duration::from_secs(2)).await;
            let screens = wayland.layout().expect("no whole layout").outputs().len();
            let on_screen = OnScreen::of(&wayland);
            assert_eq!(on_screen.found(), Ok(()));
            let (views, shown) = tokio::sync::watch::channel(view(Activity::Idle));
            tokio::task::spawn_local(on_screen.watch(shown));
            // Twelve to nine: across a change of the surfaces' size.
            for seconds in (9..=12).rev() {
                views.send_replace(view(Activity::StartingIn(seconds)));
                every_screen_shows(&on_screen, screens, &seconds.to_string()).await;
                sleep(Duration::from_secs(1)).await;
            }
            // The run begins: the number gives way to the badge, then each
            // badge in each corner.
            for (activity, text) in [
                (Activity::Clicking, "pointer"),
                (Activity::Recording, "dot"),
                (Activity::Playing, "triangle"),
            ] {
                for corner in Corner::ALL {
                    views.send_replace(on_screen_view(activity, corner));
                    every_screen_shows(&on_screen, screens, text).await;
                    sleep(Duration::from_secs(1)).await;
                }
            }
            views.send_replace(view(Activity::Idle));
            let gone = timeout(Duration::from_secs(1), async {
                while !on_screen.screens().is_empty() {
                    yield_now().await;
                }
            })
            .await;
            assert!(gone.is_ok(), "{:?}", on_screen.screens());
        })
        .await;
}
