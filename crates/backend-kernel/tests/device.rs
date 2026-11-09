//! Hand-run test against real devices and a real Wayland window.
//!
//! Needs write access to `/dev/uinput`, read access to `/dev/input/event*`
//! (the `input` group), a Wayland session (Plasma 6.8 is the target) and
//! Python with GTK 4 (`python-gobject`, `gtk4`). It opens a fullscreen window
//! and drives the pointer and keyboard for a few seconds: leave both alone
//! while it runs. From the repository root, inside the session:
//!
//! ```text
//! cargo test --locked -p auto-clicker-backend-kernel --test device -- --ignored
//! ```

// clippy.toml exempts `#[test]` functions only; this whole file is test code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use auto_clicker_backend_kernel::{DeviceRole, KernelBackend};
use auto_clicker_core::input::{AbsolutePosition, Button, Input, Key};

/// Generous: a window or a device can take a moment to appear.
const LINE_TIMEOUT: Duration = Duration::from_secs(10);

struct Window {
    child: Child,
    lines: Receiver<String>,
}

impl Window {
    fn open() -> Window {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/probe_window.py");
        let mut child = Command::new("python3")
            .arg(script)
            .stdout(Stdio::piped())
            .spawn()
            .expect("start the probe window (python3 with GTK 4)");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Window { child, lines }
    }

    /// Waits for the first line that `accept` takes, skipping others.
    fn expect<T>(&self, what: &str, accept: impl Fn(&str) -> Option<T>) -> T {
        let deadline = Instant::now() + LINE_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("window never reported {what}"));
            if let Some(found) = accept(&line) {
                return found;
            }
        }
    }

    fn expect_line(&self, wanted: &str) {
        self.expect(wanted, |line| (line == wanted).then_some(()));
    }

    fn try_motion(&self, wait: Duration) -> Option<(i64, i64)> {
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if let Some(found) = parse_motion(&self.lines.recv_timeout(left).ok()?) {
                return Some(found);
            }
        }
    }

    /// Drops lines that arrived so far.
    fn drain(&self) {
        while self.lines.try_recv().is_ok() {}
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn parse_motion(line: &str) -> Option<(i64, i64)> {
    let mut words = line.strip_prefix("motion ")?.split(' ');
    let x = words.next()?.parse().ok()?;
    let y = words.next()?.parse().ok()?;
    Some((x, y))
}

/// The middle of the output layout.
const CENTER: AbsolutePosition = AbsolutePosition { x: 32768, y: 32768 };

/// The compositor adds a new device a moment after it appears and says
/// nothing when it has, so move until the window sees motion. Alternating
/// between two points makes every attempt a real move.
fn wait_for_pointer(input: &mut Input<KernelBackend>, window: &Window) {
    let points = [CENTER, AbsolutePosition { x: 16384, y: 32768 }];
    let deadline = Instant::now() + LINE_TIMEOUT;
    for point in points.iter().cycle() {
        assert!(
            Instant::now() < deadline,
            "the compositor never moved the pointer for the virtual mouse"
        );
        input.move_absolute(*point).unwrap();
        if window.try_motion(Duration::from_millis(250)).is_some() {
            return;
        }
    }
}

/// Moves right by a relative amount and waits until the window sees it.
fn nudge_right(input: &mut Input<KernelBackend>, window: &Window, from_x: i64) -> i64 {
    window.drain();
    input.move_relative(80, 0).unwrap();
    window
        .expect("motion to the right", |line| {
            parse_motion(line).filter(|&(x, _)| x > from_x)
        })
        .0
}

fn key(code: u16) -> Key {
    Key::new(code).expect("a keyboard key")
}

const KEY_1: u16 = 2;
const KEY_ENTER: u16 = 28;
const KEY_A: u16 = 30;
const KEY_C: u16 = 46;

fn tap(input: &mut Input<KernelBackend>, code: u16) {
    input.press_key(key(code)).unwrap();
    input.release_key(key(code)).unwrap();
}

fn click(input: &mut Input<KernelBackend>, window: &Window, button: Button, number: u8) {
    input.press_button(button).unwrap();
    window.expect_line(&format!("press {number}"));
    input.release_button(button).unwrap();
    window.expect_line(&format!("release {number}"));
}

fn syspath(input: &Input<KernelBackend>, role: DeviceRole) -> PathBuf {
    input
        .backend()
        .devices()
        .iter()
        .find(|identity| identity.role == role)
        .map(|identity| identity.syspath.clone())
        .expect("both devices have an identity")
}

/// The kernel's own view of which keys and buttons are down on the device.
fn keys_down(syspath: &Path) -> Vec<u16> {
    let event = std::fs::read_dir(syspath)
        .unwrap()
        .map_while(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .find(|name| name.starts_with("event"))
        .expect("the input node has an event device");
    let device = evdev::Device::open(Path::new("/dev/input").join(event)).unwrap();
    device
        .get_key_state()
        .unwrap()
        .iter()
        .map(|k| k.0)
        .collect()
}

#[test]
#[ignore = "needs /dev/uinput, a Wayland session and Python with GTK 4; run by hand"]
fn clicks_and_types_into_a_real_window_and_leaves_nothing_pressed() {
    let mut input = KernelBackend::open().expect("create the virtual devices");
    let mouse = syspath(&input, DeviceRole::Mouse);
    let keyboard = syspath(&input, DeviceRole::Keyboard);
    let window = Window::open();
    window.expect_line("ready");

    // Absolute motion lands in the fullscreen window, relative motion moves
    // right, and moving back to the same point is not dropped as a repeat.
    wait_for_pointer(&mut input, &window);
    let away = nudge_right(&mut input, &window, i64::MIN);
    window.drain();
    input.move_absolute(CENTER).unwrap();
    let (center_x, _) = window.expect("motion to the center", |line| {
        parse_motion(line).filter(|&(x, _)| x != away)
    });
    nudge_right(&mut input, &window, center_x);
    window.drain();
    input.move_absolute(CENTER).unwrap();
    window.expect("motion back to the center", |line| {
        parse_motion(line).filter(|&(x, _)| x == center_x)
    });

    click(&mut input, &window, Button::Left, 1);
    click(&mut input, &window, Button::Right, 3);

    // One notch down scrolls down; positive notches are up.
    input.scroll(-1).unwrap();
    window.expect("a scroll down", |line| {
        let steps: i64 = line.strip_prefix("scroll ")?.parse().ok()?;
        (steps > 0).then_some(())
    });

    for code in [KEY_A, KEY_C, KEY_1, KEY_ENTER] {
        tap(&mut input, code);
    }
    window.expect_line("text ac1");

    // Held input shows in the kernel's key state, and release_all clears it.
    input.press_button(Button::Left).unwrap();
    input.press_key(key(KEY_A)).unwrap();
    window.expect_line("press 1");
    assert_eq!(keys_down(&mouse), [Button::Left.code()]);
    assert_eq!(keys_down(&keyboard), [KEY_A]);
    input.release_all().unwrap();
    window.expect_line("release 1");
    assert_eq!(keys_down(&mouse), Vec::<u16>::new());
    assert_eq!(keys_down(&keyboard), Vec::<u16>::new());

    // Dropping while a button is held releases it, and the devices go away.
    input.press_button(Button::Left).unwrap();
    window.expect_line("press 1");
    drop(input);
    window.expect_line("release 1");
    assert!(!mouse.exists() && !keyboard.exists());
}
