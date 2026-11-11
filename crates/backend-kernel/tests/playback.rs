//! Hand-run tests of playback against real kernel devices.
//!
//! Need write access to `/dev/uinput`, read access to `/dev/input/event*`
//! (the `input` group), a Wayland session and Python with GTK 4: like the
//! device test, they open the fullscreen probe window, so what is played
//! lands there. Leave the mouse and keyboard alone while they run. From the
//! repository root, inside the session:
//!
//! ```text
//! cargo test --locked -p auto-clicker-backend-kernel --test playback -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! The last two tests observe and print: one measures how far replayed
//! motion moves the pointer, the other what a window sees of a key held on a
//! real keyboard while playback presses the same key. What they print goes
//! in the README ("Playing a recording"). Neither needs the `input` group.

// clippy.toml exempts `#[test]` functions only; this whole file is test code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::pending;
use std::io::{BufRead, BufReader};
use std::num::NonZeroU32;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc as std_mpsc;
use std::thread;
use std::time::Duration;

use auto_clicker_backend_kernel::{
    InputSources, KernelBackend, local_time, monotonic_now, read_input,
};
use auto_clicker_core::clicker::Repeat;
use auto_clicker_core::hotkeys::{Binding, Bindings, HotkeyEvent, HotkeyStatus};
use auto_clicker_core::input::{AbsolutePosition, Input, Key};
use auto_clicker_core::playback::{Motion, Playback, PlaybackSettings, Speed, play};
use auto_clicker_core::recording::{
    Change, Created, Ending, Event, Recorder, Recording, RecordingSlot, recordable,
};
use auto_clicker_core::run::{Ended, Signals};
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{sleep, timeout};

/// Generous: a window or a device takes a moment to appear and be opened.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Long enough for the reader and the compositor to have seen what was sent.
const SETTLE: Duration = Duration::from_millis(300);

const KEY_A: u16 = 30;
const KEY_W: u16 = 17;
const BTN_LEFT: u16 = 0x110;

/// Hotkeys the played recording never presses.
fn bindings() -> Bindings {
    Bindings {
        toggle_clicking: Binding::parse("Ctrl+F24").unwrap(),
        toggle_recording: Binding::parse("Ctrl+F23").unwrap(),
        toggle_playback: Binding::parse("Ctrl+F22").unwrap(),
    }
}

/// The probe window, which takes what is played; killed on drop.
struct Window(Child, std_mpsc::Receiver<String>);

impl Window {
    fn open() -> Window {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/probe_window.py");
        let mut child = Command::new("python3")
            .arg(script)
            .stdout(Stdio::piped())
            .spawn()
            .expect("start the probe window (python3 with GTK 4)");
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = std_mpsc::channel();
        thread::spawn(move || {
            let mut read = BufReader::new(stdout).lines().map_while(Result::ok);
            while read.next().is_some_and(|line| tx.send(line).is_ok()) {}
        });
        // Owned from here, so a window that never gets ready is killed.
        let window = Window(child, lines);
        let ready = || window.1.recv_timeout(TIMEOUT).ok();
        while ready().expect("the window never got ready") != "ready" {}
        window
    }

    /// What the window reported until it went quiet for [`SETTLE`], or for
    /// a second if it never did (a held key repeats for as long as it is
    /// held).
    fn settled(&self) -> Vec<String> {
        let end = std::time::Instant::now() + Duration::from_secs(1);
        let next = || {
            let left = end.checked_duration_since(std::time::Instant::now())?;
            self.1.recv_timeout(left.min(SETTLE)).ok()
        };
        std::iter::from_fn(next).collect()
    }

    /// The pointer's x in the window, from the last motion reported before
    /// the window went quiet; `None` when it reported none.
    fn settled_x(&self) -> Option<i64> {
        let x_of = |line: &String| -> Option<i64> {
            let rest = line.strip_prefix("motion ")?;
            rest.split(' ').next()?.parse().ok()
        };
        self.settled().iter().filter_map(x_of).next_back()
    }

    /// The key presses and releases among what the window reported, a run
    /// of the same line (a key repeating) given once.
    fn settled_keys(&self) -> Vec<String> {
        let keys = |line: &String| line.starts_with("key");
        let mut lines: Vec<String> = self.settled().into_iter().filter(keys).collect();
        lines.dedup();
        lines
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn recording(duration_ms: u64, events: &[(u64, Change)]) -> Recording {
    let events = events
        .iter()
        .map(|&(at_ms, change)| Event {
            at: at_ms * 1000,
            change,
        })
        .collect();
    let created = Created::new(local_time().unwrap()).unwrap();
    Recording::new(
        "device test".to_owned(),
        created,
        duration_ms * 1000,
        events,
    )
    .unwrap()
}

fn playback(recording: Recording, speed: f64, times: u32) -> Playback {
    let settings = PlaybackSettings {
        speed: Speed::from_ratio(speed).unwrap(),
        repeat: Repeat::Times(NonZeroU32::new(times).unwrap()),
    };
    Playback::new(recording, settings).unwrap()
}

/// Plays `playback` to its end on `input`, and hands the input back.
async fn play_through(input: Input<KernelBackend>, playback: &Playback) -> Input<KernelBackend> {
    let signals = Signals {
        stop: pending(),
        shutdown: pending(),
    };
    let end = play(input, playback, &Motion::Relative, signals).await;
    assert!(
        matches!(end.result, Ok(Ended::Finished)),
        "{:?}",
        end.result
    );
    end.input.expect("the input comes back")
}

/// A routine with one of everything: keys, a click, motion and the wheel,
/// with gaps that are not whole milliseconds apart in the played schedule.
fn routine() -> Recording {
    let (w, a, left) = (KEY_W, KEY_A, BTN_LEFT);
    let down = |code| Change::Press(recordable(code).unwrap());
    let up = |code| Change::Release(recordable(code).unwrap());
    recording(
        600,
        &[
            (0, down(w)),
            (37, Change::Move { dx: 12, dy: -7 }),
            (61, down(left)),
            (95, up(left)),
            (140, Change::Wheel(-1)),
            (203, down(a)),
            (251, up(a)),
            (318, Change::Move { dx: -5, dy: 9 }),
            (377, Change::Wheel(2)),
            (450, up(w)),
        ],
    )
}

/// Each event's change and its time after the first event, in microseconds.
fn since_first(events: &[Event]) -> Vec<(Change, u64)> {
    let first = events.first().map_or(0, |event| event.at);
    events
        .iter()
        .map(|event| (event.change, event.at - first))
        .collect()
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs /dev/uinput, the input group and a Wayland session with GTK 4; run by hand"]
async fn a_recording_replays_through_real_devices_with_its_timing() {
    // Twice, at one and a half times the speed: 400 ms a round.
    const ROUND_US: u64 = 400_000;
    let _window = Window::open();
    let input = KernelBackend::open().unwrap();
    let playback = playback(routine(), 1.5, 2);

    // A reader that does not know the program's own devices records them,
    // with the kernel's own timestamps.
    let slot = RecordingSlot::new();
    let (events, mut received) = mpsc::unbounded_channel();
    let (stop, stopped) = oneshot::channel::<()>();
    let reader = read_input(
        InputSources {
            bindings: bindings(),
            own: &[],
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
        let available = HotkeyEvent::Status(HotkeyStatus::Available);
        timeout(TIMEOUT, async {
            while received.recv().await.as_ref() != Some(&available) {}
        })
        .await
        .expect("the keyboards never became readable");
        let created = Created::new(local_time().unwrap()).unwrap();
        slot.start(Recorder::new(bindings(), monotonic_now(), created, None));
        // The reader opens the mice once the recording has started.
        sleep(SETTLE).await;

        let input = play_through(input, &playback).await;
        sleep(SETTLE).await;
        let recorder = slot.take().unwrap();
        let replayed = recorder.finish(Ending::At(monotonic_now())).unwrap();
        stop.send(()).unwrap();
        drop(input);
        replayed.expect("nothing was recorded")
    };
    let ((), replayed) = tokio::join!(reader, test);

    let original = since_first(routine().events());
    let expected: Vec<(Change, u64)> = (0..2)
        .flat_map(|round| {
            original
                .iter()
                .map(move |&(change, at)| (change, round * ROUND_US + at * 2 / 3))
        })
        .collect();
    let replayed = since_first(replayed.events());
    let changes = |events: &[(Change, u64)]| -> Vec<Change> {
        events.iter().map(|(change, _)| *change).collect()
    };
    assert_eq!(changes(&replayed), changes(&expected));
    let errors: Vec<i64> = replayed
        .iter()
        .zip(&expected)
        .map(|(replayed, expected)| replayed.1 as i64 - expected.1 as i64)
        .collect();
    let worst = errors.iter().map(|error| error.abs()).max().unwrap();
    println!("replayed against recorded, in microseconds: {errors:?}");
    println!("worst: {worst} us over {} events", errors.len());
    // A few milliseconds: the timer's one, and a busy machine's scheduling.
    assert!(worst < 5_000, "an event was {worst} us off");
}

/// 400 units to the right, as `steps` moves `gap_ms` apart.
fn sweep(steps: u64, gap_ms: u64) -> Recording {
    let dx = i32::try_from(400 / steps).unwrap();
    let moves: Vec<(u64, Change)> = (0..steps)
        .map(|step| (step * gap_ms, Change::Move { dx, dy: 0 }))
        .collect();
    recording(steps * gap_ms, &moves)
}

/// Puts the pointer near the window's left edge, in the layout's middle
/// row, and returns its x there. The window is on one output of maybe
/// several, so this sweeps the layout from the left until the window sees
/// the pointer; `nudge` shifts the sweep, so that a second call moves the
/// pointer even when it ends where the first one did.
fn to_the_left(input: &mut Input<KernelBackend>, window: &Window, nudge: u16) -> i64 {
    // The compositor adds a new device a moment after it appears, hence
    // more than one sweep.
    for _ in 0..4 {
        for step in 0..32u16 {
            let x = 500 + nudge + step * 2000;
            input
                .move_absolute(AbsolutePosition { x, y: 32768 })
                .unwrap();
            if let Some(x) = window.settled_x() {
                return x;
            }
        }
    }
    panic!("the compositor never moved the pointer in the window");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs /dev/uinput and a Wayland session with GTK 4; prints its measurements; run by hand"]
async fn replayed_motion_moves_the_pointer_and_its_distance_is_measured() {
    let window = Window::open();
    let mut input = KernelBackend::open().unwrap();
    println!("400 units of relative motion to the right, in window pixels:");
    let mut nudge = 0;
    // Slow and fast hand movement, each at three playing speeds.
    for (steps, gap_ms, what) in [(100, 8, "500 units/s"), (20, 2, "10000 units/s")] {
        for speed in [0.25, 1.0, 4.0] {
            nudge = 300 - nudge;
            let from = to_the_left(&mut input, &window, nudge);
            input = play_through(input, &playback(sweep(steps, gap_ms), speed, 1)).await;
            let to = window.settled_x().expect("the pointer never moved");
            let moved = to - from;
            println!(
                "  recorded at {what}, played at speed {speed}: {moved} px ({:.2} px per unit)",
                moved as f64 / 400.0
            );
            assert!(moved > 0, "the pointer moved {moved} px");
        }
    }
}

/// A keyboard that is not this program's: it stands for a real one.
fn real_keyboard() -> VirtualDevice {
    let mut keys = AttributeSet::<KeyCode>::new();
    for code in [KEY_A, KEY_W] {
        keys.insert(KeyCode(code));
    }
    let builder = VirtualDevice::builder().unwrap();
    let builder = builder.name("auto-clicker test keyboard");
    builder.with_keys(&keys).unwrap().build().unwrap()
}

fn key(keyboard: &mut VirtualDevice, code: u16, value: i32) {
    let event = InputEvent::new(EventType::KEY.0, code, value);
    keyboard.emit(&[event]).unwrap();
}

/// Taps W on both keyboards until the window has seen each: the compositor
/// adds a new device a moment after it appears.
fn until_both_type(input: &mut Input<KernelBackend>, real: &mut VirtualDevice, window: &Window) {
    let w = Key::new(KEY_W).unwrap();
    let seen = |window: &Window| !window.settled_keys().is_empty();
    let ours = (0..20).any(|_| {
        input.press_key(w).unwrap();
        input.release_key(w).unwrap();
        seen(window)
    });
    let theirs = (0..20).any(|_| {
        key(real, KEY_W, 1);
        key(real, KEY_W, 0);
        seen(window)
    });
    assert!(
        ours && theirs,
        "the window never saw a key from both keyboards"
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs /dev/uinput and a Wayland session with GTK 4; prints what it sees; run by hand"]
async fn a_key_held_by_the_user_while_playback_presses_it_is_observed() {
    let window = Window::open();
    let mut input = KernelBackend::open().unwrap();
    let mut real = real_keyboard();
    until_both_type(&mut input, &mut real, &window);
    let a = recordable(KEY_A).unwrap();
    let tap = recording(300, &[(50, Change::Press(a)), (150, Change::Release(a))]);

    println!("A held on a real keyboard while playback presses and releases A:");
    key(&mut real, KEY_A, 1);
    println!("  the user presses A:      {:?}", window.settled_keys());
    let input = play_through(input, &playback(tap, 1.0, 1)).await;
    let during = window.settled_keys();
    println!("  playback taps A:         {during:?}");
    key(&mut real, KEY_A, 0);
    let after = window.settled_keys();
    println!("  the user lets go of A:   {after:?}");
    drop(input);

    // Whatever the window saw in between, A is up once both let go.
    let last = during.iter().chain(&after).next_back();
    assert_eq!(last.map(String::as_str), Some("keyup 30"));
}
