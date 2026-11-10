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
//! shows: answer Deny.

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
/// output's index in `layout`.
async fn find_window_output(
    input: &mut Input<PortalBackend>,
    window: &mut Window,
    layout: &Layout,
) -> usize {
    for _ in 0..20 {
        for (index, region) in layout.regions().iter().enumerate() {
            let (w, h) = (f64::from(region.width), f64::from(region.height));
            let center =
                layout.position_of(f64::from(region.x) + w / 2.0, f64::from(region.y) + h / 2.0);
            window.drain().await;
            input.move_absolute(center).unwrap();
            let seen = window.find(Duration::from_millis(300), parse_motion).await;
            if let Some((x, y)) = seen {
                assert!(
                    (x - w / 2.0).abs() <= 1.0 && (y - h / 2.0).abs() <= 1.0,
                    "output {index} ({region:?}): the middle arrived at {x},{y}"
                );
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
    let own_bottom = f64::from(region.y) + f64::from(region.height);
    if own_bottom >= bottom {
        println!("skipped the point below a shorter output: the window is on the tallest one");
        return;
    }
    let w = f64::from(region.width);
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

    // Relative motion moves by exactly the step, with no acceleration.
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
#[ignore = "needs Plasma 6.8, the installed desktop file and a person to click Deny; run by hand"]
async fn deny_in_the_dialog_is_not_allowed() {
    let opened = PortalOutput::new(None).open().await;
    assert!(
        matches!(opened, Err(InputError::NotAllowed)),
        "{:?}",
        opened.err()
    );
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
