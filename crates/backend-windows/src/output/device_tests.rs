//! Output into real windows, in the Windows test VM: each test starts the
//! receiver (its own test binary again, in a process of its own) and sends
//! through [`Output`]. Run one at a time, in the desktop:
//!
//! `vm run --timeout 900 --workdir C:\work\src -- cargo test --locked -p
//! auto-clicker-backend-windows -- --ignored --test-threads=1`

mod receiver;

use std::io::{BufRead, BufReader, Write};
use std::num::NonZeroU32;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use auto_clicker_core::clicker::{ClickerSettings, Interval, Mode, Repeat, run};
use auto_clicker_core::input::{AbsolutePosition, Button, Input, Key, PressState};
use auto_clicker_core::run::{Ended, Signals};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
};

use super::*;

/// Set in the receiver's environment, so its test serves instead of
/// returning at once.
const RECEIVER: &str = "AUTO_CLICKER_WN04_RECEIVER";

/// How long the tests wait for the receiver's window or its lines.
const PATIENCE: Duration = Duration::from_secs(60);

#[test]
#[ignore = "the receiver of the other device tests; runs only when they start it"]
fn receiver() {
    if std::env::var_os(RECEIVER).is_some() {
        receiver::serve();
    }
}

/// A running receiver: its window is in front.
struct Receiver {
    child: Child,
    input: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
}

impl Receiver {
    fn start() -> Receiver {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "output::device_tests::receiver",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(RECEIVER, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                // The harness may have begun the line ("test ... ").
                match line.find("rx ").map(|at| &line[at + 3..]) {
                    Some(line) => {
                        let _ = send.send(line.to_owned());
                    }
                    // The harness's own lines, shown for a receiver that
                    // fails.
                    None => eprintln!("receiver: {line}"),
                }
            }
        });
        let input = child.stdin.take();
        let receiver = Receiver {
            child,
            input,
            lines,
        };
        let ready = receiver.lines.recv_timeout(PATIENCE).unwrap();
        assert!(ready.starts_with("ready"), "{ready}");
        receiver
    }

    /// Closes the receiver's window and returns every line it noted.
    fn finish(mut self) -> Vec<String> {
        drop(self.input.take());
        let mut noted = Vec::new();
        loop {
            let line = self.lines.recv_timeout(PATIENCE).unwrap();
            if line == "end" {
                break;
            }
            noted.push(line);
        }
        assert!(self.child.wait().unwrap().success());
        noted
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        if let Some(mut input) = self.input.take() {
            let _ = input.flush();
            let _ = self.child.kill();
        }
    }
}

fn screen() -> (u32, u32) {
    // SAFETY: plain queries.
    #[allow(unsafe_code)]
    let (width, height) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    (width.cast_unsigned(), height.cast_unsigned())
}

#[allow(unsafe_code)]
fn cursor() -> (i32, i32) {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` lives for the call.
    assert_ne!(unsafe { GetCursorPos(&raw mut point) }, 0);
    (point.x, point.y)
}

/// Core's position for pixel `offset` of `size`: `round(offset * 65536 /
/// size)`, as `Layout::absolute` gives it.
fn position(offset: u32, size: u32) -> u16 {
    let scaled = (u64::from(offset) * 65_536 + u64::from(size) / 2) / u64::from(size);
    u16::try_from(scaled.min(65_535)).unwrap()
}

/// Moves the pointer to the middle of the receiver's window.
fn aim_at_receiver(output: &mut Output) {
    let (x, y, width, height) = receiver::RECT;
    let (screen_width, screen_height) = screen();
    let middle = AbsolutePosition {
        x: position((x + width / 2).cast_unsigned(), screen_width),
        y: position((y + height / 2).cast_unsigned(), screen_height),
    };
    output.move_absolute(middle).unwrap();
    thread::sleep(Duration::from_millis(100));
}

fn output() -> Output {
    Output { _private: () }
}

/// The marker as the receiver prints extra information.
fn marked() -> String {
    format!("{:#x}", MARKER)
}

/// The taps a key gets: lock keys twice, so each ends as it started.
fn taps(key: Key) -> usize {
    let locks = ["capslock", "numlock", "scrolllock"];
    if key.name().is_some_and(|name| locks.contains(&name)) {
        2
    } else {
        1
    }
}

/// Every press and release `keys` make, as `(down, name)`.
fn presses(keys: &[Key]) -> Vec<(bool, Option<&'static str>)> {
    keys.iter()
        .flat_map(|&key| [(true, key.name()), (false, key.name())].repeat(taps(key)))
        .collect()
}

/// A receiver's key line as `(down, name)`, after checking its marker.
fn received(line: &str) -> (bool, Option<&'static str>) {
    let fields: Vec<&str> = line.split(' ').collect();
    let number = |text: &str| u16::from_str_radix(text.trim_start_matches("0x"), 16).unwrap();
    assert_eq!(fields[6], marked(), "{line}");
    let scan = u8::try_from(number(fields[3])).unwrap();
    let key = received_key(scan, fields[4] == "1", number(fields[5]));
    (fields[1] == "down", key.and_then(Key::name))
}

#[test]
#[ignore = "sends keys into a window: Windows test VM"]
fn every_named_key_reaches_a_window_as_itself_with_the_marker() {
    // PrintScreen and the Windows keys act on Windows itself and take the
    // foreground; they are left out.
    let skipped = ["sysrq", "leftmeta", "rightmeta"];
    let keys: Vec<Key> = Key::CODES
        .filter_map(Key::new)
        .filter(|key| key.name().is_some_and(|name| !skipped.contains(&name)))
        .collect();
    assert_eq!(keys.len(), 114);
    let receiver = Receiver::start();
    let mut output = output();
    for &key in &keys {
        for _ in 0..taps(key) {
            output.key(key, PressState::Down).unwrap();
            output.key(key, PressState::Up).unwrap();
            thread::sleep(Duration::from_millis(5));
        }
    }
    let noted = receiver.finish();

    let keyed: Vec<&String> = noted
        .iter()
        .filter(|line| line.starts_with("key "))
        .collect();
    let actual: Vec<(bool, Option<&str>)> = keyed.iter().map(|line| received(line)).collect();
    assert_eq!(actual, presses(&keys), "{keyed:#?}");
}

#[test]
#[ignore = "clicks and scrolls into a window: Windows test VM"]
fn every_button_and_the_wheel_reach_a_window_with_the_marker() {
    let receiver = Receiver::start();
    let mut output = output();
    aim_at_receiver(&mut output);
    for button in Button::ALL {
        output.button(button, PressState::Down).unwrap();
        output.button(button, PressState::Up).unwrap();
        thread::sleep(Duration::from_millis(20));
    }
    output.scroll(1).unwrap();
    output.scroll(-2).unwrap();
    thread::sleep(Duration::from_millis(100));
    let noted = receiver.finish();

    let marker = marked();
    let mut seen = Vec::new();
    for line in &noted {
        let fields: Vec<&str> = line.split(' ').collect();
        match fields[0] {
            "button" => {
                assert_eq!(fields[4], marker, "{line}");
                seen.push(format!("{} {}", fields[2], fields[1]));
            }
            "wheel" => {
                assert_eq!(fields[3], marker, "{line}");
                seen.push(format!("wheel {}", fields[2]));
            }
            _ => {}
        }
    }
    let expected = [
        "left down",
        "left up",
        "right down",
        "right up",
        "middle down",
        "middle up",
        "x1 down",
        "x1 up",
        "x2 down",
        "x2 up",
        "wheel 120",
        "wheel -240",
    ];
    assert_eq!(seen, expected, "{noted:#?}");
}

#[test]
#[ignore = "moves the pointer: Windows test VM"]
fn relative_moves_move_the_pointer_their_way() {
    let mut output = output();
    aim_at_receiver(&mut output);
    let before = cursor();
    output.move_relative(40, 0).unwrap();
    thread::sleep(Duration::from_millis(100));
    let after = cursor();
    assert!(
        after.0 > before.0 && after.1 == before.1,
        "{before:?} to {after:?}"
    );
}

/// Sends `position` and returns where the pointer landed once it moved, or
/// where it is after a short wait.
fn land(output: &mut Output, position: AbsolutePosition, from: (i32, i32)) -> (i32, i32) {
    output.move_absolute(position).unwrap();
    for _ in 0..20 {
        let now = cursor();
        if now != from {
            return now;
        }
        thread::sleep(Duration::from_millis(1));
    }
    cursor()
}

#[test]
#[ignore = "moves the pointer over the whole screen: Windows test VM"]
fn absolute_moves_land_on_every_pixel_of_a_row_and_a_column() {
    let (width, height) = screen();
    let mut output = output();
    let row = height / 2;
    let column = width / 2;
    let mut misses = Vec::new();
    let mut at = cursor();
    for x in 0..width {
        let position = AbsolutePosition {
            x: position(x, width),
            y: position(row, height),
        };
        at = land(&mut output, position, at);
        if at != (x.cast_signed(), row.cast_signed()) {
            misses.push(format!("x {x} landed at {at:?}"));
        }
    }
    for y in 0..height {
        let position = AbsolutePosition {
            x: position(column, width),
            y: position(y, height),
        };
        at = land(&mut output, position, at);
        if at != (column.cast_signed(), y.cast_signed()) {
            misses.push(format!("y {y} landed at {at:?}"));
        }
    }
    println!(
        "screen {width}x{height}: {} of {} missed",
        misses.len(),
        width + height
    );
    for miss in misses.iter().take(20) {
        println!("{miss}");
    }
    assert!(misses.is_empty());
}

// Timing (C02's criterion)

/// Clicks per run, and runs per interval, as X03 measured.
const CLICKS: u32 = 1000;
const RUNS: usize = 3;

/// Periods left out at each run's start: the first press comes late and is
/// caught up (0006 point 2).
const START: usize = 3;

/// At most this many periods per run may miss by 10 % or more past its
/// start, for a rare scheduling stall (X03: 3 of 2,991 at 10 ms, all in
/// one stall).
const STALLS: usize = 10;

fn clicking(interval: Duration) -> ClickerSettings {
    ClickerSettings {
        mode: Mode::Click,
        interval: Interval::new(interval).unwrap(),
        repeat: Repeat::Times(NonZeroU32::new(CLICKS).unwrap()),
        ..ClickerSettings::default()
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

/// The 99th percentile of `values`.
fn p99(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let index = (values.len() * 99).div_ceil(100).saturating_sub(1);
    values[index]
}

#[test]
#[ignore = "clicks 9,000 times into a window and times them: Windows test VM"]
fn clicks_at_10_12_and_15_ms_arrive_on_time() {
    let intervals = [10, 12, 15].map(Duration::from_millis);
    let receiver = Receiver::start();
    let mut aim = output();
    aim_at_receiver(&mut aim);
    let runtime = runtime();
    for interval in intervals {
        for _ in 0..RUNS {
            let input = Output::open();
            let signals = Signals {
                stop: std::future::pending::<()>(),
                shutdown: std::future::pending::<()>(),
            };
            let rng = fastrand::Rng::with_seed(1);
            let end = runtime.block_on(run(input, &clicking(interval), rng, signals));
            assert_eq!(end.result.unwrap(), Ended::Finished);
            assert!(
                end.input
                    .as_ref()
                    .is_some_and(|input: &Input<Output>| input.held().next().is_none())
            );
            thread::sleep(Duration::from_millis(300));
        }
    }
    let noted = receiver.finish();

    let downs: Vec<f64> = noted
        .iter()
        .filter(|line| line.starts_with("button down left "))
        .map(|line| line.split(' ').nth(3).unwrap().parse::<f64>().unwrap() / 1000.0)
        .collect();
    let ups = noted
        .iter()
        .filter(|line| line.starts_with("button up left "))
        .count();
    let total = usize::try_from(CLICKS).unwrap() * RUNS * intervals.len();
    assert_eq!((downs.len(), ups), (total, total), "every click arrives");

    let runs = downs.chunks(usize::try_from(CLICKS).unwrap());
    let failures: Vec<usize> = runs
        .enumerate()
        .filter(|(index, run)| !meets_c02(run, intervals[index / RUNS], index % RUNS + 1))
        .map(|(index, _)| index)
        .collect();
    assert!(
        failures.is_empty(),
        "runs {failures:?} miss C02's criterion"
    );
}

/// Whether one run's presses, at `arrivals` (ms), meet C02's criterion for
/// `interval`: past the start, at most [`STALLS`] periods off by 10 % or
/// more, and a 99th-percentile error under 1 ms. Prints the measure.
fn meets_c02(arrivals: &[f64], interval: Duration, number: usize) -> bool {
    let interval = interval.as_secs_f64() * 1000.0;
    let errors: Vec<f64> = arrivals
        .windows(2)
        .map(|pair| (pair[1] - pair[0] - interval).abs())
        .collect();
    let is_off = |error: &&f64| **error >= interval / 10.0;
    let off_at_start = errors[..START].iter().filter(is_off).count();
    let mut errors = errors[START..].to_vec();
    let off = errors.iter().filter(is_off).count();
    let p99 = p99(&mut errors);
    println!(
        "{interval} ms run {number}: off {off} (+{off_at_start} at the start) of {}, \
         p99 error {p99:.3} ms",
        errors.len() + START
    );
    off <= STALLS && p99 < 1.0
}
