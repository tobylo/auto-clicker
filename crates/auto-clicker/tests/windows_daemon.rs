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

use auto_clicker_backend_windows::desktop;

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
    desktop::powershell(script, Duration::from_secs(60)).unwrap()
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

/// Brings the daemon's tray icon out of the taskbar's overflow, where
/// Windows 11 puts a new icon, as a user does once in the taskbar's
/// settings; then where it shows.
fn promoted_icon(pid: desktop::Process) -> desktop::Area {
    wait_for("tray icon", || desktop::tray_icon(pid));
    let daemon = std::path::Path::new(DAEMON).canonicalize().unwrap();
    // Windows names the program without the verbatim prefix.
    let daemon = daemon.display().to_string();
    // Quoted for PowerShell: an apostrophe is doubled.
    let daemon = daemon.trim_start_matches(r"\\?\").replace('\'', "''");
    let script = format!(
        "$n = 0; Get-ChildItem 'HKCU:\\Control Panel\\NotifyIconSettings' | ForEach-Object {{ \
         $p = Get-ItemProperty $_.PSPath; if ($p.ExecutablePath -eq '{daemon}') {{ \
         Set-ItemProperty $_.PSPath -Name IsPromoted -Value 1 -Type DWord; $n++ }} }}; $n"
    );
    wait_for("entry in the taskbar's settings", || {
        let count: u32 = powershell(&script).trim().parse().unwrap_or(0);
        (count > 0).then_some(())
    });
    sleep(Duration::from_secs(1));
    wait_for("tray icon", || desktop::tray_icon(pid))
}

/// Waits for `ready` to give something, at most [`START`].
fn wait_for<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + START;
    loop {
        if let Some(found) = ready() {
            return found;
        }
        assert!(Instant::now() < deadline, "no {what} in {START:?}");
        sleep(Duration::from_millis(50));
    }
}

/// The tray popup's text, read with the pointer resting on `icon`, once it
/// satisfies `wanted`.
fn popup_once(pid: desktop::Process, icon: desktop::Area, wanted: impl Fn(&str) -> bool) -> String {
    let on_icon = icon.centre();
    wait_for("popup", || {
        // Off the icon and back, so the taskbar says the pointer arrived.
        desktop::move_pointer(on_icon.down(-300)).unwrap();
        sleep(Duration::from_millis(100));
        desktop::move_pointer(on_icon).unwrap();
        let shown = Instant::now();
        while shown.elapsed() < Duration::from_secs(2) {
            if let Some(text) = desktop::popup_text(pid).filter(|text| wanted(text)) {
                return Some(text);
            }
            sleep(Duration::from_millis(50));
        }
        None
    })
}

/// Opens the tray's menu and chooses the line labelled `label`, in the
/// submenu `under` when given.
fn choose(icon: desktop::Area, under: Option<&str>, label: &str) {
    println!("choosing {label}");
    // A click within a double click's time of the menu closing is taken
    // for the click that closed it, and opens nothing.
    sleep(REOPEN);
    desktop::click(icon.centre(), desktop::Button::Left).unwrap();
    let mut menu = wait_for("menu", desktop::shown_menu);
    if let Some(under) = under {
        let place = place_of(&menu, under);
        desktop::move_pointer(menu.area(place).unwrap().centre()).unwrap();
        menu = wait_for("submenu", || {
            desktop::shown_menu().filter(|shown| shown.lines().iter().all(|l| l.label != under))
        });
    }
    menu.click(place_of(&menu, label)).unwrap();
    wait_for("menu to close", || {
        desktop::shown_menu().is_none().then_some(())
    });
}

fn place_of(menu: &desktop::Menu, label: &str) -> u32 {
    let lines = menu.lines();
    let place = lines
        .iter()
        .position(|line| line.label == label)
        .unwrap_or_else(|| panic!("no {label:?} in {lines:?}"));
    u32::try_from(place).unwrap()
}

/// Longer than a double click's time (500 ms unless set otherwise).
const REOPEN: Duration = Duration::from_secs(1);

/// A recording the Play menu lists, named with an ampersand, which the menu
/// shows as written.
const RECORDING: &str = "2026-11-09_18-00-00.000.json";
const RECORDED: &str = r#"{"version":1,"name":"Farm & loop","created":"2026-11-09T18:00:00.000+01:00","duration_us":1000000,"events":[
[0,"wheel",-1],
[1000000,"wheel",1]
]}"#;
/// Its line in the Play menu, as the menu holds it.
const RECORDING_LINE: &str = "Farm && loop (2026-11-09 18:00:00)";

#[test]
#[ignore = "starts the daemon in the desktop and clicks its tray icon: run in the WN01 VM through `vm run`"]
fn every_tray_item_sends_what_it_sends_on_linux() {
    let home = Home::new("tray");
    let recordings = home.0.join(r"local\auto-clicker\recordings");
    std::fs::create_dir_all(&recordings).unwrap();
    std::fs::write(recordings.join(RECORDING), RECORDED).unwrap();
    let mut daemon = home.start();
    home.running(&mut daemon);
    let pid = desktop::Process(daemon.id());
    let icon = promoted_icon(pid);
    let idle = |text: &str| text.starts_with("Auto clicker: idle");
    popup_once(pid, icon, idle);
    // The menu as on Linux, Settings off until the window comes.
    desktop::click(icon.centre(), desktop::Button::Left).unwrap();
    let menu = wait_for("menu", desktop::shown_menu);
    let lines: Vec<(String, bool)> = menu
        .lines()
        .into_iter()
        .map(|line| (line.label, line.greyed))
        .collect();
    let line = |label: &str, greyed| (label.to_owned(), greyed);
    assert_eq!(
        lines,
        [
            line("Start clicking", false),
            // No positions until WN09 reads the pointer: as on Linux, it
            // says so.
            line("Record (movement only)", false),
            line("Play", false),
            line("Settings (not on Windows yet)", true),
            line("", true),
            line("Quit", false),
        ]
    );
    desktop::press(0x1b).unwrap();
    wait_for("menu to close", || {
        desktop::shown_menu().is_none().then_some(())
    });
    // Start: a countdown, stopped by Stop clicking.
    choose(icon, None, "Start clicking");
    popup_once(pid, icon, |text| {
        text.starts_with("Auto clicker: starts in")
    });
    choose(icon, None, "Stop clicking");
    popup_once(pid, icon, idle);
    // Started and left to run: no output to open until the watchdog comes.
    choose(icon, None, "Start clicking");
    popup_once(pid, icon, |text| idle(text) && text.contains("watchdog"));
    // Record, then Stop recording.
    choose(icon, None, "Record (movement only)");
    popup_once(pid, icon, |text| text.starts_with("Auto clicker: record"));
    choose(icon, None, "Stop recording");
    popup_once(pid, icon, |text| !text.starts_with("Auto clicker: record"));
    // The recording in the Play menu, then Stop playing.
    choose(icon, Some("Play"), RECORDING_LINE);
    popup_once(pid, icon, |text| text.starts_with("Auto clicker: plays in"));
    choose(icon, None, "Stop playing");
    popup_once(pid, icon, idle);
    // Quit ends the daemon, and its icon goes with it.
    let owner = desktop::icon_owner(pid).unwrap();
    choose(icon, None, "Quit");
    assert!(ended(&mut daemon, END).success(), "{}", home.log());
    assert_eq!(owner.area(), None, "the icon outlived the daemon");
}
