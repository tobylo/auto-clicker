//! Hand-run test of the single instance against the real daemon.
//!
//! Needs write access to `/dev/uinput`, a session bus, and no auto-clicker
//! running already. The daemon it starts clicks nothing: it is never asked
//! to. From the repository root:
//!
//! ```text
//! cargo test --locked -p auto-clicker --test instance -- --ignored --test-threads=1
//! ```

// clippy.toml exempts `#[test]` functions only; this whole file is test code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use auto_clicker_backend_kernel::KernelBackend;

const DAEMON: &str = env!("CARGO_BIN_EXE_auto-clicker");
const BUS_NAME: &str = "io.github.tobylo.AutoClicker";
/// Generous: the daemon only connects to the bus and opens two devices.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// The kernel's numbers of the input devices a new pair of virtual devices
/// gets (`/sys/devices/virtual/input/inputN`). The kernel counts them up and
/// never reuses one, so a gap means another device was created in between.
fn probe_numbers() -> Vec<u64> {
    let input = KernelBackend::open().unwrap();
    input
        .backend()
        .devices()
        .iter()
        .map(|device| {
            let name = device.syspath.file_name().unwrap().to_str().unwrap();
            name.strip_prefix("input").unwrap().parse().unwrap()
        })
        .collect()
}

fn name_is_owned() -> bool {
    Command::new("busctl")
        .args(["--user", "status", BUS_NAME])
        .output()
        .unwrap()
        .status
        .success()
}

/// Whether the daemon has started: its tray item is registered after its
/// devices are open and its signal handlers installed.
fn has_started(daemon: &Child) -> bool {
    let item = format!("org.kde.StatusNotifierItem-{}-", daemon.id());
    let names = Command::new("busctl")
        .args(["--user", "list", "--no-legend"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&names.stdout).contains(&item)
}

fn wait_for_start(daemon: &mut Child) {
    let deadline = Instant::now() + START_TIMEOUT;
    while !has_started(daemon) {
        assert!(
            daemon.try_wait().unwrap().is_none(),
            "the daemon exited at start"
        );
        assert!(Instant::now() < deadline, "the daemon never started");
        sleep(Duration::from_millis(50));
    }
}

/// The daemon under test; killed on drop if the test failed before it was
/// ended, so a failing test leaves no daemon behind.
struct Daemon(Option<Child>);

impl Daemon {
    fn start() -> Daemon {
        Daemon(Some(Command::new(DAEMON).spawn().unwrap()))
    }

    fn child(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }

    /// Sends SIGTERM and waits, at most `START_TIMEOUT`, for the exit status.
    fn terminate(&mut self) -> Option<i32> {
        let mut child = self.0.take().unwrap();
        let pid = child.id().to_string();
        Command::new("kill").args(["-TERM", &pid]).status().unwrap();
        match exit_within(&mut child, START_TIMEOUT) {
            Some(code) => code,
            None => {
                kill(child);
                panic!("the daemon did not exit after SIGTERM");
            }
        }
    }
}

/// The exit code once `child` has exited, or `None` if it is still running
/// after `limit`.
fn exit_within(child: &mut Child, limit: Duration) -> Option<Option<i32>> {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status.code());
        }
        if Instant::now() > deadline {
            return None;
        }
        sleep(Duration::from_millis(50));
    }
}

/// Kills `child` and reaps it, waiting at most `START_TIMEOUT`.
fn kill(mut child: Child) {
    let _ = child.kill();
    if exit_within(&mut child, START_TIMEOUT).is_none() {
        eprintln!("daemon {} survived SIGKILL", child.id());
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(child) = self.0.take() {
            kill(child);
        }
    }
}

/// Runs a second daemon and returns its exit code and stderr. Killed if it
/// does not exit in time: it would be a second running daemon.
fn second_daemon() -> (Option<i32>, String) {
    let mut child = Command::new(DAEMON).stderr(Stdio::piped()).spawn().unwrap();
    let Some(code) = exit_within(&mut child, START_TIMEOUT) else {
        kill(child);
        panic!("the second daemon kept running");
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    (code, stderr)
}

#[test]
#[ignore = "needs /dev/uinput, a session bus and no running auto-clicker"]
fn a_second_daemon_exits_before_opening_any_device() {
    assert!(!name_is_owned(), "an auto-clicker is already running");
    let mut first = Daemon::start();
    wait_for_start(first.child());
    let before = probe_numbers();
    let second = second_daemon();
    let after = probe_numbers();
    let first_status = first.terminate();
    assert_eq!(second.0, Some(3));
    assert_eq!(second.1.trim(), "auto-clicker is already running");
    let next = before.iter().max().unwrap() + 1;
    assert_eq!(
        after.iter().min(),
        Some(&next),
        "the second daemon created a device"
    );
    assert_eq!(first_status, Some(0), "SIGTERM ends the daemon cleanly");
}
