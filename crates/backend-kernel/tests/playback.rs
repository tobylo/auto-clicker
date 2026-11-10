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
//! The second test measures and prints; it asserts only that the pointer
//! moved. Its numbers go in the README ("Playing a recording").

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
use auto_clicker_core::input::{AbsolutePosition, Input};
use auto_clicker_core::playback::{Playback, PlaybackSettings, Speed, play};
use auto_clicker_core::recording::{
    Change, Created, Ending, Event, Recorder, Recording, RecordingSlot, recordable,
};
use auto_clicker_core::run::{Ended, Signals};
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

    /// The pointer's x in the window, from the last motion reported before
    /// the window went quiet for [`SETTLE`]; `None` when it reported none.
    fn settled_x(&self) -> Option<i64> {
        let mut last = None;
        while let Ok(line) = self.1.recv_timeout(SETTLE) {
            let x = line
                .strip_prefix("motion ")
                .and_then(|rest| rest.split(' ').next()?.parse().ok());
            last = x.or(last);
        }
        last
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
    let end = play(input, playback, signals).await;
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

/// Puts the pointer near the window's left edge, in its middle row, and
/// returns its x there.
fn to_the_left(input: &mut Input<KernelBackend>, window: &Window) -> i64 {
    // The compositor adds a new device a moment after it appears, so move
    // until the window sees it; two points make every attempt a real move.
    let points = [(6000, 32768), (5000, 32768)];
    for (x, y) in points.into_iter().cycle().take(40) {
        input.move_absolute(AbsolutePosition { x, y }).unwrap();
        if let Some(x) = window.settled_x() {
            return x;
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
    // Slow and fast hand movement, each at three playing speeds.
    for (steps, gap_ms, what) in [(100, 8, "500 units/s"), (20, 2, "10000 units/s")] {
        for speed in [0.25, 1.0, 4.0] {
            let from = to_the_left(&mut input, &window);
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
