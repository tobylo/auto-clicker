//! Hand-run test of recording against real kernel devices.
//!
//! Needs write access to `/dev/uinput`, read access to `/dev/input/event*`
//! (the `input` group), a Wayland session and Python with GTK 4: like the
//! device test, it opens the fullscreen probe window, so the clicks and keys
//! it sends land there. It creates a throwaway virtual mouse and keyboard
//! that stand for real ones, records them, and checks the recording. Leave
//! the mouse and keyboard alone while it runs. From the repository root,
//! inside the session:
//!
//! ```text
//! cargo test --locked -p auto-clicker-backend-kernel --test recording -- --ignored --test-threads=1
//! ```

// clippy.toml exempts `#[test]` functions only; this whole file is test code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc as std_mpsc;
use std::thread;
use std::time::Duration;

use auto_clicker_backend_kernel::{
    InputSources, KernelBackend, local_time, monotonic_now, read_input,
};
use auto_clicker_core::hotkeys::{Action, Binding, Bindings, HotkeyEvent, HotkeyStatus};
use auto_clicker_core::input::{Held, Input, Key};
use auto_clicker_core::recording::{
    Change, Created, Ending, Recorder, Recording, RecordingSlot, recordable,
};
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode, RelativeAxisCode};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep, timeout};

/// Generous: a device takes a moment to appear and be opened.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Long enough for the reader to have read what was just sent.
const SETTLE: Duration = Duration::from_millis(100);
/// Between events on different devices.
const GAP: Duration = Duration::from_millis(5);

const LEFTCTRL: u16 = 29;
const KEY_A: u16 = 30;
const KEY_C: u16 = 46;
const KEY_B: u16 = 48;
const F23: u16 = 193;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;

fn bindings() -> Bindings {
    Bindings {
        toggle_clicking: Binding::parse("Ctrl+F24").unwrap(),
        toggle_recording: Binding::parse("Ctrl+F23").unwrap(),
        toggle_playback: Binding::parse("Ctrl+F22").unwrap(),
    }
}

/// The probe window, which takes the clicks and keys; killed on drop. Its
/// lines are read until then, so it never writes to a closed pipe.
struct Window(Child, std_mpsc::Receiver<String>);

impl Window {
    fn open() -> Window {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/probe_window.py");
        let mut child = Command::new("python3")
            .arg(script)
            .stdout(Stdio::piped())
            .spawn()
            .expect("start the probe window (python3 with GTK 4)");
        let lines = forward_lines(child.stdout.take().unwrap());
        // Owned from here, so a window that never gets ready is killed.
        let window = Window(child, lines);
        let ready = || window.1.recv_timeout(TIMEOUT).ok();
        while ready().expect("the window never got ready") != "ready" {}
        window
    }
}

/// The lines `stdout` prints, as they come.
fn forward_lines(stdout: std::process::ChildStdout) -> std_mpsc::Receiver<String> {
    let (tx, lines) = std_mpsc::channel();
    thread::spawn(move || {
        let mut read = BufReader::new(stdout).lines().map_while(Result::ok);
        while read.next().is_some_and(|line| tx.send(line).is_ok()) {}
    });
    lines
}

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

/// A virtual device that is not this program's: it stands for a real one.
fn stand_in(name: &str, keys: &[u16], mouse: bool) -> VirtualDevice {
    let mut set = AttributeSet::<KeyCode>::new();
    for &code in keys {
        set.insert(KeyCode(code));
    }
    let builder = VirtualDevice::builder()
        .unwrap()
        .name(name)
        .with_keys(&set)
        .unwrap();
    let builder = if mouse {
        let mut axes = AttributeSet::<RelativeAxisCode>::new();
        for axis in [
            RelativeAxisCode::REL_X,
            RelativeAxisCode::REL_Y,
            RelativeAxisCode::REL_WHEEL,
        ] {
            axes.insert(axis);
        }
        builder.with_relative_axes(&axes).unwrap()
    } else {
        builder
    };
    builder.build().unwrap()
}

fn node(device: &mut VirtualDevice) -> PathBuf {
    device
        .enumerate_dev_nodes_blocking()
        .unwrap()
        .find_map(Result::ok)
        .expect("the device has no event node")
}

fn key(device: &mut VirtualDevice, code: u16, value: i32) {
    device
        .emit(&[InputEvent::new(EventType::KEY.0, code, value)])
        .unwrap();
}

fn relative(device: &mut VirtualDevice, axis: RelativeAxisCode, value: i32) {
    device
        .emit(&[InputEvent::new(EventType::RELATIVE.0, axis.0, value)])
        .unwrap();
}

/// Whether this process has `node` open.
fn has_open(node: &Path) -> bool {
    fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .any(|fd| fs::read_link(fd.path()).is_ok_and(|target| target == node))
}

/// Waits until `done` holds, checking every few milliseconds.
async fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        sleep(Duration::from_millis(10)).await;
    }
}

async fn next_hotkey_event(
    received: &mut mpsc::UnboundedReceiver<HotkeyEvent>,
    wanted: &HotkeyEvent,
) {
    timeout(TIMEOUT, async {
        while received.recv().await.as_ref() != Some(wanted) {}
    })
    .await
    .unwrap_or_else(|_| panic!("never received {wanted:?}"));
}

fn held(code: u16) -> Held {
    recordable(code).unwrap()
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs /dev/uinput, the input group and a Wayland session with GTK 4; run by hand"]
async fn records_stand_in_devices_but_never_the_programs_own() {
    let _window = Window::open();
    let mut own = KernelBackend::open().unwrap();
    let identities = own.backend().devices().to_vec();
    let mut keyboard = stand_in(
        "auto-clicker test keyboard",
        &[LEFTCTRL, KEY_A, KEY_B, F23],
        false,
    );
    let mut mouse = stand_in("auto-clicker test mouse", &[BTN_LEFT, BTN_RIGHT], true);
    let unplugged = stand_in("auto-clicker test mouse 2", &[BTN_LEFT, BTN_RIGHT], true);
    let mouse_node = node(&mut mouse);

    let slot = RecordingSlot::new();
    let (events, mut received) = mpsc::unbounded_channel();
    let (stop, stopped) = oneshot::channel::<()>();
    let reader = read_input(
        InputSources {
            capture: None,
            bindings: bindings(),
            own: &identities,
            feed: slot.feed(),
        },
        move |event| {
            let _ = events.send(event);
        },
        async move {
            let _ = stopped.await;
        },
    );
    let test = async {
        next_hotkey_event(&mut received, &HotkeyEvent::Status(HotkeyStatus::Available)).await;
        sleep(SETTLE).await;
        assert!(!has_open(&mouse_node), "a mouse is read while idle");

        // B and the left button are already held when recording starts.
        key(&mut keyboard, KEY_B, 1);
        key(&mut mouse, BTN_LEFT, 1);
        sleep(SETTLE).await;
        let created = Created::new(local_time().unwrap()).unwrap();
        slot.start(Recorder::new(bindings(), monotonic_now(), created, None));
        until("the mouse is read", || has_open(&mouse_node)).await;
        sleep(SETTLE).await;
        play_routine(&mut own, &mut keyboard, &mut mouse, unplugged).await;

        // Stopped by the hotkey, as the controller would.
        key(&mut keyboard, LEFTCTRL, 1);
        key(&mut keyboard, F23, 1);
        next_hotkey_event(&mut received, &HotkeyEvent::Fired(Action::ToggleRecording)).await;
        let recorder = slot.take().unwrap();
        key(&mut keyboard, F23, 0);
        key(&mut keyboard, LEFTCTRL, 0);
        let recording = recorder
            .finish(Ending::At(monotonic_now()))
            .unwrap()
            .unwrap();
        until("the mouse is closed again", || !has_open(&mouse_node)).await;
        stop.send(()).unwrap();
        recording
    };
    let ((), recording) = tokio::join!(reader, test);
    check(&recording);
}

/// What the recording sees: motion, the program's own input, a click, the
/// wheel, keys with autorepeat, and a mouse unplugged with a button down.
async fn play_routine(
    own: &mut Input<KernelBackend>,
    keyboard: &mut VirtualDevice,
    mouse: &mut VirtualDevice,
    mut unplugged: VirtualDevice,
) {
    key(mouse, BTN_LEFT, 0);
    for _ in 0..3 {
        relative(mouse, RelativeAxisCode::REL_X, 5);
        sleep(Duration::from_millis(3)).await;
    }
    // The program's own devices: never recorded.
    own.move_relative(7, 0).unwrap();
    own.press_key(Key::new(KEY_C).unwrap()).unwrap();
    own.release_key(Key::new(KEY_C).unwrap()).unwrap();
    sleep(SETTLE).await;
    key(mouse, BTN_LEFT, 1);
    key(mouse, BTN_LEFT, 0);
    relative(mouse, RelativeAxisCode::REL_WHEEL, -1);
    // Events on different devices a moment apart, so their order is not
    // left to equal timestamps.
    sleep(GAP).await;
    for value in [1, 2, 0] {
        key(keyboard, KEY_A, value);
    }
    key(keyboard, KEY_B, 0);
    sleep(GAP).await;
    key(&mut unplugged, BTN_RIGHT, 1);
    sleep(SETTLE).await;
    drop(unplugged);
    sleep(SETTLE).await;
    // More than the kernel queues for a reader, sent without letting it
    // read: the mouse loses events. That must not stop the hotkeys.
    for _ in 0..5_000 {
        relative(mouse, RelativeAxisCode::REL_Y, 1);
    }
    sleep(SETTLE).await;
}

fn check(recording: &Recording) {
    let changes: Vec<Change> = recording
        .events()
        .iter()
        .map(|event| event.change)
        .filter(|change| !matches!(change, Change::Move { .. }))
        .collect();
    let moved = recording
        .events()
        .iter()
        .fold((0, 0), |(x, y), event| match event.change {
            Change::Move { dx, dy } => (x + dx, y + dy),
            _ => (x, y),
        });
    assert_eq!(
        changes,
        [
            Change::Press(held(KEY_B)),
            Change::Press(held(BTN_LEFT)),
            Change::Release(held(BTN_LEFT)),
            Change::Press(held(BTN_LEFT)),
            Change::Release(held(BTN_LEFT)),
            Change::Wheel(-1),
            Change::Press(held(KEY_A)),
            Change::Release(held(KEY_A)),
            Change::Release(held(KEY_B)),
            Change::Press(held(BTN_RIGHT)),
            Change::Release(held(BTN_RIGHT)),
        ]
    );
    assert_eq!(moved.0, 15, "the program's own motion was recorded");
    // Released when unplugged, well before the stop.
    let unplugged = recording
        .events()
        .iter()
        .find(|event| event.change == Change::Release(held(BTN_RIGHT)))
        .unwrap();
    assert!(unplugged.at + 100_000 <= recording.duration_us());
    let first = recording.events().first().unwrap();
    assert!(
        first.at < 50_000,
        "B held at the start came at {} us",
        first.at
    );
}
