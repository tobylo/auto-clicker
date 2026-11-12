//! Against the running compositor: run by hand on a Wayland session with
//! `cargo test --locked -p auto-clicker-backend-wayland -- --ignored`.

use super::*;

#[tokio::test]
#[ignore = "needs a running Wayland compositor that offers xdg-output"]
async fn the_running_compositor_gives_a_layout_and_its_listener() {
    let wayland = tokio::time::timeout(std::time::Duration::from_secs(2), Wayland::connect())
        .await
        .expect("the display did not answer within 2 s");
    assert_eq!(wayland.missing(), None);
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

    let wayland = timeout(Duration::from_secs(2), Wayland::connect())
        .await
        .expect("the display did not answer within 2 s");
    let before = wayland.layout();
    eprintln!("before: {before:?}; change a screen's scale or place now");
    let changed = LayoutSignal::default();
    let watch = wayland.watch(changed.clone());
    tokio::select! {
        () = watch => panic!("the connection ended"),
        fired = timeout(Duration::from_secs(60), changed.fired()) => {
            fired.expect("no change within 60 s");
        }
    }
    eprintln!("after: {:?}", wayland.layout());
    assert_ne!(wayland.layout(), before);
}
