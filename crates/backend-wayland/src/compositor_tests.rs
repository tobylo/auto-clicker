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
