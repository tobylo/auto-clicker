//! Recordings with positions in the daemon (WN09): positions play only on
//! the system that took them, on the screens they lie on; a recording from
//! Linux plays its relative motion, with the notice.

use super::*;

/// Where the pointer is put before each playback.
const START_AT: desktop::Point = desktop::Point { x: 1000, y: 700 };

/// A recording on the primary screen whose positions `source` took
/// (`"kwin"` for version 2, else version 3's `positions_from`): at
/// (600, 400), then 40 counts right at 100 ms with its position, then F13
/// held to 19 s, so it still plays when the tray is read; the test stops it.
fn positioned(source: &str) -> String {
    let (width, height) = desktop::screen_size();
    let (version, from) = match source {
        "kwin" => (2, String::new()),
        named => (3, format!(r#","positions_from":"{named}""#)),
    };
    format!(
        r#"{{"version":{version},"name":"From {source}","created":"2026-11-27T12:00:00.000+01:00","duration_us":20000000{from},"layout":[{{"x":0,"y":0,"width":{width},"height":{height},"scale_milli":1000}}],"events":[
[0,"at",600,400],
[100000,"move",40,0],
[100000,"at",640,400],
[200000,"press","KEY_F13"],
[19000000,"release","KEY_F13"]
]}}"#
    )
}

/// A daemon of its own holding only `recording`, its hotkeys registered.
fn holding_recording(what: &str, recording: &str) -> (Home, Daemon) {
    let home = Home::new(what);
    home.config("version = 1\ntray_start_delay = 0\n");
    let recordings = home.0.join(r"local\auto-clicker\recordings");
    std::fs::create_dir_all(&recordings).unwrap();
    std::fs::write(recordings.join("2026-11-27_12-00-00.000.json"), recording).unwrap();
    let mut daemon = home.start();
    home.running(&mut daemon);
    wait_for("the hotkeys", || {
        desktop::HeldHotkey::taken(VK_F6).then_some(())
    });
    (home, daemon)
}

/// Plays the daemon's recording by its hotkey from [`START_AT`]; where the
/// pointer is once its moves have played.
fn played_to() -> desktop::Point {
    desktop::move_pointer(START_AT).unwrap();
    assert_eq!(desktop::pointer().unwrap(), START_AT);
    combination(F8);
    // Its moves are done within 100 ms of the start; the start waits for
    // the hotkey's modifiers, which are up already.
    sleep(Duration::from_secs(1));
    desktop::pointer().unwrap()
}

#[test]
#[ignore = "plays recordings into the desktop's pointer: run in the WN01 VM through `vm run`"]
fn positions_play_only_on_the_system_that_took_them() {
    // Taken on Windows, on these screens: the positions play.
    let (_home, daemon) = holding_recording("positions-windows", &positioned("windows"));
    assert_eq!(
        played_to(),
        desktop::Point { x: 640, y: 400 },
        "a Windows recording's positions"
    );
    let icon = promoted_icon(daemon.process());
    let text = popup_once(daemon.process(), icon, |text| text.contains("From windows"));
    assert!(
        !text.contains("relative") && !text.contains("relativa"),
        "{text}"
    );
    combination(F8);
    drop(daemon);

    // Taken on Linux, on screens of the same size: relative motion, and
    // the notice says the positions are not used.
    for source in ["kwin", "hyprland"] {
        let (_home, daemon) = holding_recording(source, &positioned(source));
        let at = played_to();
        assert!(at.x > START_AT.x && at.y == START_AT.y, "{source}: {at:?}");
        let icon = promoted_icon(daemon.process());
        popup_once(daemon.process(), icon, |text| {
            text.contains("Playing with relative motion: a screen this recording used")
                || text.contains("Spelar upp med relativa rörelser: en skärm som inspelningen")
        });
        combination(F8);
    }
}
