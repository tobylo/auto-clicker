//! Hand-run tests against the real portal and a real Wayland window.
//!
//! They need Plasma 6.8 on Wayland, the package's
//! `io.github.tobylo.AutoClicker.desktop` installed (the portal grants by
//! app id), Python with GTK 4 (`python-gobject`, `gtk4`), and a person at the
//! machine: the first run shows KDE's permission dialog. They open a
//! fullscreen window and drive the pointer and keyboard for a few seconds:
//! leave both alone while they run. From the repository root, inside the
//! session, one at a time:
//!
//! ```text
//! cargo test --locked -p auto-clicker-backend-portal plasma -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! `clicks_through_the_portal_into_a_real_window` uses the daemon's own
//! token file: answer Allow if the dialog shows; a second run shows none.
//! `deny_in_the_dialog_is_not_allowed` keeps no token, so the dialog always
//! shows: answer Deny. `a_hotkey_pressed_by_a_person_arrives_through_the_portal`
//! binds the daemon's own shortcuts: accept the dialog if it shows, then
//! press the hotkey it names.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use auto_clicker_core::input::{AbsolutePosition, Button, Input, InputError, Key};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::layout::Layout;
use crate::{PortalBackend, PortalOutput, token_path};

/// Generous: a window or a device can take a moment to appear.
const LINE_TIMEOUT: Duration = Duration::from_secs(10);
/// A restore from the token answered in 7 to 11 ms in X02; a dialog takes
/// a person seconds.
const NO_DIALOG: Duration = Duration::from_secs(2);

/// The probe window the kernel backend's device test uses too.
struct Window {
    child: Child,
    lines: mpsc::UnboundedReceiver<String>,
}

impl Window {
    fn open() -> Window {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../backend-kernel/tests/fixtures/probe_window.py");
        let mut child = Command::new("python3")
            .arg(script)
            .stdout(Stdio::piped())
            .spawn()
            .expect("start the probe window (python3 with GTK 4)");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, lines) = mpsc::unbounded_channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Window { child, lines }
    }

    /// The first line within `wait` that `accept` takes, skipping others.
    async fn find<T>(&mut self, wait: Duration, accept: impl Fn(&str) -> Option<T>) -> Option<T> {
        timeout(wait, async {
            while let Some(line) = self.lines.recv().await {
                if let Some(found) = accept(&line) {
                    return Some(found);
                }
            }
            None
        })
        .await
        .ok()
        .flatten()
    }

    async fn expect_line(&mut self, wanted: &str) {
        self.find(LINE_TIMEOUT, |line| (line == wanted).then_some(()))
            .await
            .unwrap_or_else(|| panic!("the window never reported {wanted:?}"));
    }

    /// Drops the lines that arrived so far.
    async fn drain(&mut self) {
        self.find(Duration::from_millis(100), |_| None::<()>).await;
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn parse_motion(line: &str) -> Option<(f64, f64)> {
    let mut words = line.strip_prefix("motion ")?.split(' ');
    let x = words.next()?.parse().ok()?;
    let y = words.next()?.parse().ok()?;
    Some((x, y))
}

/// The `/dev/input` event nodes: libei must add none.
fn event_nodes() -> Vec<String> {
    let mut nodes: Vec<String> = std::fs::read_dir("/dev/input")
        .unwrap()
        .map_while(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("event"))
        .collect();
    nodes.sort();
    nodes
}

/// Opens a session with the daemon's token. If the dialog showed (no token
/// yet), the open says so and a second one must go through without it.
async fn open_with_token() -> Input<PortalBackend> {
    let output = PortalOutput::new(token_path());
    let started = Instant::now();
    match output.open().await {
        Ok(input) => {
            println!("restored from the token in {:?}", started.elapsed());
            input
        }
        Err(InputError::AllowedNow) => {
            println!("the dialog showed and was allowed; opening again");
            let again = Instant::now();
            let input = output.open().await.expect("the second open");
            let took = again.elapsed();
            assert!(took < NO_DIALOG, "the dialog showed again ({took:?})");
            println!("restored from the new token in {took:?}");
            input
        }
        Err(err) => panic!("open: {err}"),
    }
}

/// Moves to the middle of each output in turn. The fullscreen window covers
/// one of them and must see the pointer at exactly its middle; returns that
/// output's index in `layout`. Each output gets a point beside its middle
/// first: a pointer that jumps into a window from another output enters it
/// and moves only on the next step, and the window reports motion only.
async fn find_window_output(
    input: &mut Input<PortalBackend>,
    window: &mut Window,
    layout: &Layout,
) -> usize {
    for _ in 0..10 {
        for (index, region) in layout.regions().iter().enumerate() {
            let (w, h) = (f64::from(region.width), f64::from(region.height));
            let (x, y) = (f64::from(region.x) + w / 2.0, f64::from(region.y) + h / 2.0);
            input
                .move_absolute(layout.position_of(x - 50.0, y - 50.0))
                .unwrap();
            input.move_absolute(layout.position_of(x, y)).unwrap();
            // The step beside the middle may show too; wait for the middle.
            // A middle that lands anywhere else never matches, and the test
            // fails below.
            let (mid_x, mid_y) = (w / 2.0, h / 2.0);
            let seen = window
                .find(Duration::from_millis(500), |line| {
                    parse_motion(line)
                        .filter(|&(x, y)| (x - mid_x).abs() <= 1.0 && (y - mid_y).abs() <= 1.0)
                })
                .await;
            if seen.is_some() {
                return index;
            }
        }
    }
    panic!("the pointer never reached the window on any output");
}

/// Below a shorter output, inside the layout's box but on no output: the
/// point moves onto that output's last row.
async fn check_below_a_shorter_output(
    input: &mut Input<PortalBackend>,
    window: &mut Window,
    layout: &Layout,
    index: usize,
) {
    let region = layout.regions()[index];
    let bottom = layout
        .regions()
        .iter()
        .map(|r| f64::from(r.y) + f64::from(r.height))
        .fold(0.0, f64::max);
    let w = f64::from(region.width);
    let (left, top) = (f64::from(region.x), f64::from(region.y));
    let target = (left + w / 2.0, bottom - 1.0);
    let on_an_output = layout.regions().iter().any(|r| {
        let (rx, ry) = (f64::from(r.x), f64::from(r.y));
        (rx..rx + f64::from(r.width)).contains(&target.0)
            && (ry..ry + f64::from(r.height)).contains(&target.1)
    });
    // The clamp must take the point onto this window's output, not another.
    let (cx, cy) = layout.point(layout.position_of(target.0, target.1));
    let lands_here = (f64::from(cx) - target.0).abs() <= 1.0
        && (f64::from(cy) - (top + f64::from(region.height) - 1.0)).abs() <= 1.0;
    if on_an_output || !lands_here {
        println!("skipped the point below a shorter output: none below the window's output");
        return;
    }
    window.drain().await;
    input
        .move_absolute(layout.position_of(f64::from(region.x) + w / 2.0, bottom - 1.0))
        .unwrap();
    let (x, y) = window
        .find(LINE_TIMEOUT, parse_motion)
        .await
        .expect("motion below the output");
    let last_row = f64::from(region.height) - 1.0;
    assert!(
        (x - w / 2.0).abs() <= 1.0 && (y - last_row).abs() <= 1.0,
        "a point below the output arrived at {x},{y}, not on its last row"
    );
    println!("a point below the shorter output landed on its last row");
}

async fn click(input: &mut Input<PortalBackend>, window: &mut Window, button: Button, number: u8) {
    input.press_button(button).unwrap();
    window.expect_line(&format!("press {number}")).await;
    input.release_button(button).unwrap();
    window.expect_line(&format!("release {number}")).await;
}

fn key(code: u16) -> Key {
    Key::new(code).expect("a keyboard key")
}

async fn drive(input: &mut Input<PortalBackend>, window: &mut Window) {
    let layout = input.backend().eis.layout().expect("the outputs");
    println!("outputs: {:?}", layout.regions());
    let index = find_window_output(input, window, &layout).await;
    println!("the window is on output {index}");
    check_below_a_shorter_output(input, window, &layout, index).await;

    // GTK numbers the buttons 1 left, 2 middle, 3 right, 8 back, 9 forward.
    for (button, number) in [
        (Button::Left, 1),
        (Button::Middle, 2),
        (Button::Right, 3),
        (Button::Side, 8),
        (Button::Extra, 9),
    ] {
        click(input, window, button, number).await;
    }

    // One notch down scrolls down; positive notches are up.
    input.scroll(-1).unwrap();
    window
        .find(LINE_TIMEOUT, |line| {
            let steps: i64 = line.strip_prefix("scroll ")?.parse().ok()?;
            (steps > 0).then_some(())
        })
        .await
        .expect("a scroll down");

    // Key codes, not text, so the keyboard layout does not matter.
    for code in [46_u16, 2, 30] {
        input.press_key(key(code)).unwrap();
        input.release_key(key(code)).unwrap();
        window.expect_line(&format!("key {code}")).await;
    }

    // An absolute point lands exactly, checked apart from the search for the
    // window's output; then relative motion moves by exactly the step, with
    // no acceleration.
    window.drain().await;
    input
        .move_absolute(layout.position_of(
            f64::from(layout.regions()[index].x) + 100.0,
            f64::from(layout.regions()[index].y) + 100.0,
        ))
        .unwrap();
    let from = window
        .find(LINE_TIMEOUT, parse_motion)
        .await
        .expect("motion");
    assert!(
        (from.0 - 100.0).abs() <= 1.0 && (from.1 - 100.0).abs() <= 1.0,
        "a move to 100,100 on the window's output arrived at {from:?}"
    );
    input.move_relative(40, 30).unwrap();
    let to = window
        .find(LINE_TIMEOUT, parse_motion)
        .await
        .expect("relative motion");
    assert_eq!((to.0 - from.0, to.1 - from.1), (40.0, 30.0));
}

#[tokio::test]
#[ignore = "needs Plasma 6.8, the installed desktop file, Python with GTK 4 and a person for the dialog; run by hand"]
async fn clicks_through_the_portal_into_a_real_window() {
    let nodes = event_nodes();
    let mut window = Window::open();
    window.expect_line("ready").await;
    let mut input = open_with_token().await;
    assert_eq!(event_nodes(), nodes, "libei added an evdev node");

    let lost = PortalOutput::lost(&input);
    tokio::select! {
        err = lost => panic!("the session ended: {err}"),
        () = drive(&mut input, &mut window) => {}
    }

    // Dropping while a button is held releases it and ends the session.
    input.press_button(Button::Left).unwrap();
    window.expect_line("press 1").await;
    drop(input);
    window.expect_line("release 1").await;
}

#[tokio::test]
#[ignore = "needs Plasma 6.8 and the installed desktop file; run by hand"]
async fn plasma_offers_its_shortcuts_without_a_dialog() {
    // No session opens, so nothing shows; the check ends well within its
    // own deadlines.
    let offered = timeout(Duration::from_secs(30), crate::shortcuts_offered())
        .await
        .expect("the check did not end in time");
    assert!(offered.is_ok(), "{:?}", offered.err());
}

#[tokio::test]
#[ignore = "needs Plasma 6.8, the installed desktop file and shortcuts already accepted; opens System Settings; run by hand"]
async fn the_desktops_shortcut_settings_open_for_a_bound_session() {
    use auto_clicker_core::controller::{ReaderCommand, ReaderCommands};
    use auto_clicker_core::hotkeys::{Bindings, HotkeyEvent, HotkeyStatus};

    let (events, mut received) = mpsc::unbounded_channel();
    let (commands, asked) = ReaderCommands::channel();
    let reader = crate::read_shortcuts(Bindings::default(), asked, move |event| {
        let _ = events.send(event);
    });
    let check = async {
        let wait = Duration::from_secs(30);
        let mut next = async || timeout(wait, received.recv()).await.unwrap().unwrap();
        while next().await != HotkeyEvent::Status(HotkeyStatus::Available) {}
        let configure = ReaderCommand::ConfigureShortcuts {
            operation: 1,
            activation_token: None,
        };
        commands.send(configure).unwrap();
        loop {
            if let HotkeyEvent::Configured { result, .. } = next().await {
                assert_eq!(result, Ok(()));
                break;
            }
        }
    };
    tokio::select! {
        () = reader => panic!("the reader ended"),
        () = check => {}
    }
}

#[tokio::test]
#[ignore = "needs Plasma 6.8, the installed desktop file and a person to click Deny; run by hand"]
async fn deny_in_the_dialog_is_not_allowed() {
    let opened = PortalOutput::new(None).open().await;
    assert!(
        matches!(opened, Err(InputError::NotAllowed)),
        "{:?}",
        opened.err()
    );
}

#[tokio::test]
#[ignore = "needs Plasma 6.8, the installed desktop file and a person to accept the dialog and press the hotkey; run by hand"]
async fn a_hotkey_pressed_by_a_person_arrives_through_the_portal() {
    use auto_clicker_core::hotkeys::{Action, Bindings, HotkeyEvent, HotkeyStatus};
    use tokio::sync::mpsc;
    use tokio::time::timeout;

    let (events, mut received) = mpsc::unbounded_channel();
    let (_commands, asked) = auto_clicker_core::controller::ReaderCommands::channel();
    let reader = crate::read_shortcuts(Bindings::default(), asked, move |event| {
        let _ = events.send(event);
    });
    let person = async {
        eprintln!("\n>>> Accept the shortcuts dialog if Plasma shows one.");
        let wait = Duration::from_secs(120);
        let mut next = async || timeout(wait, received.recv()).await.unwrap().unwrap();
        let HotkeyEvent::Triggers(triggers) = next().await else {
            panic!("the triggers come first");
        };
        let trigger = triggers
            .of(Action::ToggleClicking)
            .text()
            .unwrap()
            .to_owned();
        assert!(
            !trigger.is_empty(),
            "no trigger for clicking: set one in System Settings"
        );
        // A trigger the program cannot read would rule out every key mode.
        let w = auto_clicker_core::input::Key::from_name("w").unwrap();
        assert_eq!(triggers.refusal(w), None, "{trigger:?} is not readable");
        assert_eq!(next().await, HotkeyEvent::Status(HotkeyStatus::Available));
        eprintln!(">>> Press {trigger} and let go.");
        assert_eq!(next().await, HotkeyEvent::Fired(Action::ToggleClicking));
        let fired = Instant::now();
        assert_eq!(next().await, HotkeyEvent::ModifiersReleased);
        eprintln!(">>> Released {:?} after the press.", fired.elapsed());
    };
    tokio::select! {
        () = reader => panic!("the reader ended"),
        () = person => {}
    }
}

/// Prints every hotkey event with its time for `PROBE_SECS` seconds (60 by
/// default): for measuring what the desktop sends when a hotkey is held,
/// let go key by key, changed in System Settings, or the portal restarts.
#[tokio::test]
#[ignore = "needs Plasma 6.8 and the installed desktop file; prints what the portal sends; run by hand"]
async fn hotkey_events_as_they_arrive() {
    use auto_clicker_core::hotkeys::Bindings;
    use tokio::time::sleep;

    let secs = std::env::var("PROBE_SECS")
        .ok()
        .and_then(|secs| secs.parse().ok())
        .unwrap_or(60);
    let start = Instant::now();
    let (_commands, asked) = auto_clicker_core::controller::ReaderCommands::channel();
    let reader = crate::read_shortcuts(Bindings::default(), asked, move |event| {
        eprintln!("{:>8.3} s  {event:?}", start.elapsed().as_secs_f64());
    });
    tokio::select! {
        () = reader => eprintln!("the reader ended"),
        () = sleep(Duration::from_secs(secs)) => {}
    }
}

#[test]
fn positions_of_points_invert_the_mapping() {
    use crate::layout::Region;
    let layout = Layout::new([
        Region {
            x: 0,
            y: 0,
            width: 2048,
            height: 1280,
        },
        Region {
            x: 2048,
            y: 0,
            width: 3440,
            height: 1440,
        },
    ])
    .unwrap();
    for (x, y) in [(1024.0, 640.0), (3768.0, 720.0), (100.0, 100.0)] {
        let (px, py) = layout.point(layout.position_of(x, y));
        assert!(
            (f64::from(px) - x).abs() < 0.1 && (f64::from(py) - y).abs() < 0.1,
            "{x},{y}: {px},{py}"
        );
    }
    assert_eq!(
        layout.position_of(-5.0, 99_999.0),
        AbsolutePosition { x: 0, y: 65535 }
    );
}
