//! Hand-run tests of the daemon on Windows, in the WN01 VM's logged-in
//! desktop: a daemon started over SSH has no desktop. Each daemon gets
//! folders of its own (`APPDATA`, `LOCALAPPDATA`), so it reads no config
//! of the VM's user and its log is the test's to read. From the dev box,
//! with the tree in `C:\work\<dir>`:
//!
//! ```text
//! scripts/windows-vm/vm run --timeout 900 --workdir 'C:\work\<dir>' -- cargo test --locked -p auto-clicker --test windows_daemon -- --ignored --test-threads=1 --nocapture
//! ```

// Windows alone, as the daemon it drives.
#![cfg(windows)]
// clippy.toml exempts `#[test]` functions only; this whole file is test code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
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
/// How many minutes are watched, at most, for one in which Windows sent
/// the daemon nothing.
const IDLE_TRIES: usize = 5;

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

    fn start(&self) -> Daemon {
        let child = Command::new(DAEMON)
            .env("APPDATA", self.0.join("roaming"))
            .env("LOCALAPPDATA", self.0.join("local"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Daemon(child)
    }

    fn log_path(&self) -> PathBuf {
        let path: PathBuf = ["local", "auto-clicker", "logs", "auto-clicker.log"]
            .iter()
            .collect();
        self.0.join(path)
    }

    /// The log so far; empty until the daemon has created it.
    fn log(&self) -> String {
        std::fs::read_to_string(self.log_path()).unwrap_or_default()
    }

    /// The log, which must be there to read: a read that fails is never
    /// taken for an empty log.
    fn existing_log(&self) -> String {
        std::fs::read_to_string(self.log_path()).unwrap()
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

/// A started daemon, ended and reaped however the test ends, a failed
/// assertion included: a daemon left running would hold the session's
/// single instance.
struct Daemon(Child);

impl Deref for Daemon {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.0
    }
}

impl DerefMut for Daemon {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // A daemon that already ended is only reaped; killing it fails.
        let _ = self.0.kill();
        let deadline = Instant::now() + END;
        while matches!(self.0.try_wait(), Ok(None)) && Instant::now() < deadline {
            sleep(Duration::from_millis(20));
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
    log.lines().filter(|line| !windows_said(line)).collect()
}

/// Whether a log line names something Windows told the daemon: a message
/// it does not act on, or a notice it heard.
fn windows_said(line: &str) -> bool {
    line.contains("woken by window message") || line.contains(": heard ")
}

/// Every thread of `pid` and how often each was switched to so far.
fn switches(pid: u32) -> BTreeMap<u32, u64> {
    let text = powershell(&format!(
        "Get-CimInstance Win32_PerfRawData_PerfProc_Thread -Filter 'IDProcess = {pid}' \
         | ForEach-Object {{ \"$($_.IDThread) $($_.ContextSwitchesPersec)\" }}"
    ));
    // Every row is read, or the sample fails: a row dropped, or none at
    // all, would measure nothing and pass.
    let sample: BTreeMap<u32, u64> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (thread, count) = line.trim().split_once(' ').expect(line);
            (thread.parse().expect(line), count.parse().expect(line))
        })
        .collect();
    assert!(!sample.is_empty(), "no thread counters for {pid}: {text:?}");
    sample
}

/// How many context switches the threads in `after` made since `before`,
/// and how many threads of `before` ended meanwhile, whose last switches
/// no sample counts.
fn wakes(before: &BTreeMap<u32, u64>, after: &BTreeMap<u32, u64>) -> (u64, usize) {
    let mut wakes = 0;
    for (thread, count) in after {
        let was = before.get(thread).copied().unwrap_or(0);
        let delta = count.saturating_sub(was);
        println!("thread {thread}: {was} -> {count} (+{delta})");
        wakes += delta;
    }
    let gone: Vec<&u32> = before
        .keys()
        .filter(|thread| !after.contains_key(thread))
        .collect();
    for thread in &gone {
        println!("thread {thread}: ended during the minute");
    }
    (wakes, gone.len())
}

#[test]
#[ignore = "starts the daemon in the desktop and watches it for up to five minutes: run in the WN01 VM through `vm run`"]
fn an_idle_daemon_wakes_for_nothing_of_its_own() {
    let home = Home::new("idle");
    let mut daemon = home.start();
    home.running(&mut daemon);
    // Let start-up settle: the loader's worker threads end on their own.
    sleep(Duration::from_secs(10));
    // A minute counts once Windows sent nothing in it and no thread ended
    // in it: then every context switch is counted and would be the
    // daemon's own. A minute with Windows' messages, or in which a thread
    // ended (the loader's workers end on their own), is watched again.
    let mut interrupted = Vec::new();
    let mut quiet = false;
    for _ in 0..IDLE_TRIES {
        let earlier = home.existing_log();
        let before = switches(daemon.id());
        sleep(IDLE);
        let after = switches(daemon.id());
        let now = home.existing_log();
        // The log only grows during the minute (a rotation would not fit
        // in it).
        let added = now
            .strip_prefix(earlier.as_str())
            .expect("the log was rewritten during the minute");
        let woken: Vec<String> = added
            .lines()
            .filter(|line| windows_said(line))
            .map(str::to_owned)
            .collect();
        // A timer, or a message the program posts to itself, is the
        // program's own wake, whatever the count says.
        let own: Vec<&String> = woken
            .iter()
            .filter(|line| line.contains("woken by window message") && own_message(line))
            .collect();
        assert!(own.is_empty(), "the daemon woke itself: {own:?}");
        let (wakes, gone) = wakes(&before, &after);
        println!("woken by Windows: {woken:?}");
        // No window message explains a wake: the daemon caused it.
        assert!(
            wakes == 0 || !woken.is_empty(),
            "{wakes} context switches in {IDLE:?} with no window message to explain them"
        );
        if woken.is_empty() && gone == 0 {
            quiet = true;
            break;
        }
        interrupted.push((wakes, gone, woken));
    }
    ask_to_quit(&daemon);
    assert!(ended(&mut daemon, END).success(), "{}", home.log());
    assert!(
        quiet,
        "Windows woke the daemon, or a thread ended, in each of {IDLE_TRIES} minutes \
         (wakes, threads ended, messages): {interrupted:?}"
    );
}

/// Whether a "woken by window message 0x...." line names a message only
/// the program itself could cause: `WM_TIMER`, or one in `WM_APP`'s range.
fn own_message(line: &str) -> bool {
    let Some(number) = line
        .rsplit("0x")
        .next()
        .and_then(|hex| u32::from_str_radix(hex.trim(), 16).ok())
    else {
        return true;
    };
    number == 0x0113 || (0x8000..0xc000).contains(&number)
}

#[test]
fn a_timer_or_a_message_to_itself_is_the_programs_own() {
    let line = |hex: &str| format!("auto-clicker: woken by window message 0x{hex}");
    assert!(own_message(&line("0113")));
    assert!(own_message(&line("8001")));
    assert!(own_message(&line("bfff")));
    assert!(!own_message(&line("001e")));
    assert!(!own_message(&line("031f")));
    // A registered message, such as Explorer's TaskbarCreated.
    assert!(!own_message(&line("c0f2")));
    assert!(own_message("garbled"));
}
