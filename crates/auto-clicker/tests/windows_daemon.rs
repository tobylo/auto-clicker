//! Hand-run tests of the daemon on Windows, in the WN01 VM's logged-in
//! desktop: a daemon started over SSH has no desktop. Each daemon gets
//! folders of its own (`APPDATA`, `LOCALAPPDATA`), so it reads no config
//! of the VM's user and its log is the test's to read. From the dev box,
//! with the tree in `C:\work\<dir>`:
//!
//! ```text
//! scripts/windows-vm/vm run --timeout 600 --workdir 'C:\work\<dir>' -- cargo test --locked -p auto-clicker --test windows_daemon -- --ignored --test-threads=1 --nocapture
//! ```

// Windows alone, as the daemon it drives.
#![cfg(windows)]
// clippy.toml exempts `#[test]` functions only; this whole file is test code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DAEMON: &str = env!("CARGO_BIN_EXE_auto-clicker");
/// Generous: the daemon creates a window and reads its config.
const START: Duration = Duration::from_secs(20);
/// How long a daemon may take to end once asked to.
const END: Duration = Duration::from_secs(10);
/// How long an idle daemon is watched (the issue's criterion).
const IDLE: Duration = Duration::from_secs(60);

/// Folders of a daemon's own, removed when dropped.
struct Home(PathBuf);

impl Home {
    fn new(what: &str) -> Home {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "auto-clicker-{what}-{}-{nanos:x}",
            std::process::id()
        ));
        std::fs::create_dir_all(dir.join("roaming")).unwrap();
        std::fs::create_dir_all(dir.join("local")).unwrap();
        Home(dir)
    }

    fn start(&self) -> Child {
        Command::new(DAEMON)
            .env("APPDATA", self.0.join("roaming"))
            .env("LOCALAPPDATA", self.0.join("local"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn log(&self) -> String {
        let path: PathBuf = ["local", "auto-clicker", "logs", "auto-clicker.log"]
            .iter()
            .collect();
        std::fs::read_to_string(self.0.join(path)).unwrap_or_default()
    }

    /// Waits until the daemon logged that it runs.
    fn running(&self, daemon: &mut Child) {
        let deadline = Instant::now() + START;
        while !self.log().contains(": running") {
            assert!(daemon.try_wait().unwrap().is_none(), "{}", self.log());
            assert!(Instant::now() < deadline, "not running: {}", self.log());
            sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn powershell(script: &str) -> String {
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .unwrap();
    assert!(output.status.success(), "{script}: {output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Waits for `child` to end, at most `within`.
fn ended(child: &mut Child, within: Duration) -> ExitStatus {
    let deadline = Instant::now() + within;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("still running after {within:?}");
        }
        sleep(Duration::from_millis(20));
    }
}

/// Asks the daemon to close its window, as `taskkill` without `/F` does.
fn ask_to_quit(daemon: &Child) {
    let status = Command::new("taskkill")
        .args(["/PID", &daemon.id().to_string()])
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "taskkill: {status}");
}

#[test]
#[ignore = "starts the daemon in the desktop: run in the WN01 VM through `vm run`"]
fn a_second_daemon_in_the_session_exits_with_the_reason() {
    let home = Home::new("second");
    let mut first = home.start();
    home.running(&mut first);
    let before = home.log();
    let mut second = home.start();
    // It says why in a message box, its only window; closing it ends it.
    let pid = second.id();
    let deadline = Instant::now() + START;
    loop {
        let title = powershell(&format!(
            "(Get-Process -Id {pid} -ErrorAction SilentlyContinue).MainWindowTitle"
        ));
        if title.trim() == "Auto Clicker" {
            break;
        }
        assert!(second.try_wait().unwrap().is_none(), "ended without a box");
        assert!(Instant::now() < deadline, "no message box");
        sleep(Duration::from_millis(100));
    }
    powershell(&format!("[void](Get-Process -Id {pid}).CloseMainWindow()"));
    assert_eq!(ended(&mut second, END).code(), Some(3));
    // The second wrote nothing: the log is the first one's alone.
    assert_eq!(said(&home.log()), said(&before));
    ask_to_quit(&first);
    assert!(ended(&mut first, END).success(), "{}", home.log());
    assert_eq!(
        said(&home.log()),
        said(&before),
        "the first quit with something to say"
    );
}

/// A log's lines but those naming what Windows woke the daemon with, which
/// come when Windows sends them.
fn said(log: &str) -> Vec<&str> {
    log.lines()
        .filter(|line| !line.contains("woken by window message"))
        .collect()
}

/// Every thread of `pid` and how often each was switched to so far.
fn switches(pid: u32) -> BTreeMap<u32, u64> {
    let text = powershell(&format!(
        "Get-CimInstance Win32_PerfRawData_PerfProc_Thread -Filter 'IDProcess = {pid}' \
         | ForEach-Object {{ \"$($_.IDThread) $($_.ContextSwitchesPersec)\" }}"
    ));
    text.lines()
        .filter_map(|line| {
            let (thread, count) = line.trim().split_once(' ')?;
            Some((thread.parse().ok()?, count.parse().ok()?))
        })
        .collect()
}

#[test]
#[ignore = "starts the daemon in the desktop and watches it for a minute: run in the WN01 VM through `vm run`"]
fn an_idle_daemon_wakes_for_nothing_of_its_own() {
    let home = Home::new("idle");
    let mut daemon = home.start();
    home.running(&mut daemon);
    // Let start-up settle: the loader's worker threads end on their own.
    sleep(Duration::from_secs(10));
    let logged = home.log().len();
    let before = switches(daemon.id());
    sleep(IDLE);
    let after = switches(daemon.id());
    let woken: Vec<String> = home.log()[logged..]
        .lines()
        .filter(|line| line.contains("woken by window message"))
        .map(str::to_owned)
        .collect();
    ask_to_quit(&daemon);
    assert!(ended(&mut daemon, END).success(), "{}", home.log());
    let mut wakes = 0;
    for (thread, count) in &after {
        let was = before.get(thread).copied().unwrap_or(0);
        let delta = count.saturating_sub(was);
        println!("thread {thread}: {was} -> {count} (+{delta})");
        wakes += delta;
    }
    for gone in before.keys().filter(|thread| !after.contains_key(thread)) {
        println!("thread {gone}: ended during the minute");
    }
    println!("woken by Windows: {woken:?}");
    // Anything that woke the daemon came from Windows and is named in the
    // log: no timer, no poll and no message to itself.
    assert!(
        wakes == 0 || !woken.is_empty(),
        "{wakes} context switches in {IDLE:?} with no window message to explain them"
    );
}
