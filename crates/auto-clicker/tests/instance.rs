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

use std::process::{Child, Command};
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

fn wait_for_name(daemon: &mut Child) {
    let deadline = Instant::now() + START_TIMEOUT;
    while !name_is_owned() {
        assert!(
            daemon.try_wait().unwrap().is_none(),
            "the daemon exited at start"
        );
        assert!(
            Instant::now() < deadline,
            "the daemon never claimed its name"
        );
        sleep(Duration::from_millis(50));
    }
}

fn terminate(daemon: Child) -> Option<i32> {
    let pid = daemon.id().to_string();
    Command::new("kill").args(["-TERM", &pid]).status().unwrap();
    daemon.wait_with_output().unwrap().status.code()
}

#[test]
#[ignore = "needs /dev/uinput, a session bus and no running auto-clicker"]
fn a_second_daemon_exits_before_opening_any_device() {
    assert!(!name_is_owned(), "an auto-clicker is already running");
    let mut first = Command::new(DAEMON).spawn().unwrap();
    wait_for_name(&mut first);
    let before = probe_numbers();
    let second = Command::new(DAEMON).output().unwrap();
    let after = probe_numbers();
    let first_status = terminate(first);
    assert_eq!(second.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&second.stderr).trim(),
        "auto-clicker is already running"
    );
    let next = before.iter().max().unwrap() + 1;
    assert_eq!(
        after.iter().min(),
        Some(&next),
        "the second daemon created a device"
    );
    assert_eq!(first_status, Some(0), "SIGTERM ends the daemon cleanly");
}
