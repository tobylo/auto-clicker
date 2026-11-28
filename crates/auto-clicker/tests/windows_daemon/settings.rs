//! The settings window on Windows (WN10), opened from the tray's Settings
//! as a user does. The window's program is built beside the daemon's
//! first: `cargo build --locked -p auto-clicker-settings`.

use std::path::{Path, PathBuf};

use super::*;

/// How long the window may take to go once its daemon, or its close
/// button, ended it: well under the session's write time (5 s).
const GONE: Duration = Duration::from_secs(3);

/// The window's program, beside the daemon's; built by the test's caller.
fn program() -> PathBuf {
    let program = Path::new(DAEMON).with_file_name("auto-clicker-settings.exe");
    assert!(
        program.exists(),
        "build it first: cargo build --locked -p auto-clicker-settings"
    );
    program
}

/// The settings window the daemon started: its process and its window,
/// once it shows.
fn shown(daemon: &Daemon) -> Option<(u32, desktop::Window)> {
    let program = program();
    let windows = children(daemon.id(), program.to_str().unwrap());
    let &[pid] = windows.as_slice() else {
        return None;
    };
    let window = *desktop::windows_of(desktop::Process(pid)).first()?;
    Some((pid, window))
}

/// The settings window, once it shows with its title.
fn titled_window(daemon: &Daemon) -> Option<(u32, desktop::Window)> {
    let title = match language() {
        Language::English => "Auto clicker settings",
        Language::Swedish => "Inställningar för autoklickaren",
    };
    shown(daemon).filter(|(_, window)| window.title() == title)
}

/// How many of the daemon's settings pipes exist now.
fn pipes(daemon: &Daemon) -> usize {
    let pid = daemon.id();
    powershell(&format!(
        "(Get-ChildItem -Path \\\\.\\pipe\\ | Where-Object Name -like 'auto-clicker-settings-{pid}-*').Count"
    ))
    .trim()
    .parse()
    .unwrap()
}

/// Waits until `pid` has ended, at most [`GONE`].
fn gone(pid: u32, why: &str) {
    let deadline = Instant::now() + GONE;
    while alive(pid) {
        assert!(Instant::now() < deadline, "the window outlived {why}");
        sleep(Duration::from_millis(20));
    }
}

/// The window's memory now, as Task Manager counts it, in KiB.
fn memory(pid: u32) -> String {
    powershell(&format!(
        "$p = Get-Process -Id {pid}; \"private $($p.PrivateMemorySize64 / 1KB) KiB, working set $($p.WorkingSet64 / 1KB) KiB\""
    ))
    .trim()
    .to_owned()
}

#[test]
#[ignore = "starts the daemon in the desktop and opens its settings window: run in the WN01 VM through `vm run`"]
fn the_window_opens_from_the_tray_once_and_nothing_outlives_it() {
    let home = Home::new("settings");
    let mut daemon = home.start();
    home.running(&mut daemon);
    let icon = promoted_icon(daemon.process());
    let chosen = Instant::now();
    choose(icon, None, label("Settings"));
    let (pid, window) = wait_for("settings window", || titled_window(&daemon));
    println!("shown {:?} after the choice", chosen.elapsed());
    wait_for("the window in front", || window.in_front().then_some(()));
    assert_eq!(pipes(&daemon), 2);
    sleep(Duration::from_secs(2));
    println!("at rest: {}", memory(pid));
    if let Some(look) = std::env::var_os("AUTO_CLICKER_TEST_LOOK") {
        let seconds = look.to_str().unwrap().parse().unwrap();
        println!("holding the window for {seconds} s");
        sleep(Duration::from_secs(seconds));
    }
    // Minimised, then chosen again: the same window comes back to the
    // front, and no second one starts.
    window.minimise().unwrap();
    wait_for("the window minimised", || {
        (window.minimised() && !window.in_front()).then_some(())
    });
    choose(icon, None, label("Settings"));
    wait_for("the window back in front", || {
        (!window.minimised() && window.in_front()).then_some(())
    });
    assert_eq!(shown(&daemon).map(|(again, _)| again), Some(pid));
    // Closed by its close button: the window and its pipes go.
    window.close().unwrap();
    gone(pid, "its close button");
    wait_for("the pipes to go", || (pipes(&daemon) == 0).then_some(()));
    // Opened again, then the daemon quits: the window goes with it.
    choose(icon, None, label("Settings"));
    let (pid, _) = wait_for("settings window", || titled_window(&daemon));
    let quit = Instant::now();
    ask_to_quit(&daemon);
    gone(pid, "the daemon's quit");
    println!("gone {:?} after the quit", quit.elapsed());
    ended(&mut daemon.watchdog, END);
}

/// Where on the window a click lands: `x` logical pixels from the left of
/// its client area, and `y` from its top, or from its bottom when negative,
/// scaled as the window is (640 logical pixels wide).
fn at(window: desktop::Window, x: i32, y: i32) -> desktop::Point {
    let area = window.client_area().unwrap();
    let scale = f64::from(area.right - area.left) / 640.0;
    let scaled = |value: i32| (f64::from(value) * scale).round() as i32;
    let from = if y < 0 { area.bottom } else { area.top };
    desktop::Point {
        x: area.left + scaled(x),
        y: from + scaled(y),
    }
}

/// Holds for a screen capture from the dev box, when the run asks for one
/// (`AUTO_CLICKER_TEST_LOOK`, seconds), saying what shows.
fn look(what: &str) {
    if let Some(seconds) = std::env::var_os("AUTO_CLICKER_TEST_LOOK") {
        let seconds: u64 = seconds.to_str().unwrap().parse().unwrap();
        println!("look: {what} for {seconds} s");
        sleep(Duration::from_secs(seconds));
    }
}

/// The tabs, by name and where their titles are.
const CLICKER: (&str, i32) = ("Clicker", 42);
const HOTKEYS: (&str, i32) = ("Hotkeys", 197);
const PLAYBACK: (&str, i32) = ("Playback", 358);
const ABOUT: (&str, i32) = ("About", 510);

fn open_tab(window: desktop::Window, (name, x): (&str, i32)) {
    desktop::click(at(window, x, 27), desktop::Button::Left).unwrap();
    look(name);
}

/// Set-1 scan codes the test types.
const CTRL: u16 = 0x1d;
const SHIFT: u16 = 0x2a;
const A: u16 = 0x1e;
const F6: u16 = 0x40;
const F7: u16 = 0x41;
const DIGITS: [u16; 10] = [0x0b, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a];

/// Presses `chord` (modifiers first) and lets it go again.
fn press(chord: &[u16]) {
    for &key in chord {
        desktop::key(key, false, true).unwrap();
    }
    for &key in chord.iter().rev() {
        desktop::key(key, false, false).unwrap();
    }
}

/// Replaces the text of the field at (`x`, `y`) with `digits`.
fn type_into(window: desktop::Window, (x, y): (i32, i32), digits: &str) {
    desktop::click(at(window, x, y), desktop::Button::Left).unwrap();
    sleep(Duration::from_millis(200));
    press(&[CTRL, A]);
    for digit in digits.chars() {
        press(&[DIGITS[digit.to_digit(10).unwrap() as usize]]);
    }
}

/// Saves, and waits until the config file holds each of `lines`.
fn save_until(home: &Home, window: desktop::Window, lines: &[&str]) {
    look("before the save");
    desktop::click(at(window, 602, -14), desktop::Button::Left).unwrap();
    let path = home.0.join(r"roaming\auto-clicker\config.toml");
    wait_for(&lines.join(", "), || {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .to_lowercase();
        lines.iter().all(|line| text.contains(line)).then_some(())
    });
}

#[test]
#[ignore = "starts the daemon in the desktop and uses its settings window: run in the WN01 VM through `vm run`"]
fn every_tab_shows_and_saves_what_it_changes() {
    let home = Home::new("tabs");
    let mut daemon = home.start();
    home.running(&mut daemon);
    let icon = promoted_icon(daemon.process());
    choose(icon, None, label("Settings"));
    let (_, window) = wait_for("settings window", || titled_window(&daemon));
    wait_for("the window in front", || window.in_front().then_some(()));
    // Clicker: the interval.
    open_tab(window, CLICKER);
    type_into(window, (322, 107), "250");
    save_until(&home, window, &["interval_ms = 250"]);
    // Hotkeys: clicking's and recording's swapped, each captured in the
    // window by pressing the other's, which Windows hands to the window
    // only while the daemon has let go of its hotkeys: a hotkey still
    // registered would go to the daemon, and the capture would wait.
    open_tab(window, HOTKEYS);
    for (row, key) in [(67, F7), (107, F6)] {
        desktop::click(at(window, 556, row), desktop::Button::Left).unwrap();
        look("capturing");
        // The capture's request reaches the daemon before the key does.
        sleep(Duration::from_millis(500));
        press(&[CTRL, SHIFT, key]);
        // Each capture is answered before the next is asked.
        sleep(Duration::from_millis(500));
    }
    let swapped = [
        "toggle_clicking = \"ctrl+shift+f7\"",
        "toggle_recording = \"ctrl+shift+f6\"",
    ];
    save_until(&home, window, &swapped);
    // Playback: the speed.
    open_tab(window, PLAYBACK);
    type_into(window, (422, 67), "2");
    save_until(&home, window, &["speed = 2.0"]);
    // About: it shows, and saves nothing.
    open_tab(window, ABOUT);
    let shown = window.title();
    assert!(!shown.is_empty());
}
