//! Hand-run tests of the daemon on Windows, in the WN01 VM's logged-in
//! desktop: a daemon started over SSH has no desktop. Each daemon gets
//! folders of its own (`APPDATA`, `LOCALAPPDATA`), so it reads no config
//! of the VM's user and its log is the test's to read. From the dev box,
//! with the tree in `C:\work\<dir>`:
//!
//! ```text
//! scripts/windows-vm/vm run --timeout 900 --workdir 'C:\work\<dir>' -- cargo test --locked -p auto-clicker --test windows_daemon -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Two tests need an elevated process and pass by otherwise; run each
//! alone through the elevated runner:
//!
//! ```text
//! scripts/windows-vm/vm run --elevated --timeout 900 --workdir 'C:\work\<dir>' -- cargo test --locked -p auto-clicker --test windows_daemon -- --ignored --exact an_elevated_watchdog_refuses_before_it_writes_or_starts_anything
//! scripts/windows-vm/vm run --elevated --timeout 900 --workdir 'C:\work\<dir>' -- cargo test --locked -p auto-clicker --test windows_daemon -- --ignored --exact a_hotkey_start_with_a_program_running_as_administrator_in_front_is_refused
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

use auto_clicker_backend_windows::desktop;
use auto_clicker_core::locale::Language;

const DAEMON: &str = env!("CARGO_BIN_EXE_auto-clicker");
const WATCHDOG: &str = env!("CARGO_BIN_EXE_auto-clicker-watchdog");
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
        // As the daemon is, so the icon's place, the pointer and the menu
        // are all in physical pixels: above 100 % an unaware driver would
        // read the per-monitor aware daemon's icon unscaled and miss it.
        // Fails harmlessly once an earlier test made the process aware.
        let _ = auto_clicker_backend_windows::become_per_monitor_aware();
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

    /// Starts the watchdog, as login does, and finds the daemon it starts.
    fn start(&self) -> Daemon {
        let watchdog = self.command(WATCHDOG).spawn().unwrap();
        // Ended and reaped from here on, should its daemon never show.
        let mut daemon = Daemon {
            watchdog,
            pid: None,
            held: None,
        };
        let id = daemon.watchdog.id();
        let pid = wait_for("the watchdog's daemon", || {
            children(id, DAEMON).first().copied()
        });
        // Still the watchdog's child once held, so the id is the daemon's.
        daemon.held = Some(
            desktop::Held::open_checked(desktop::Process(pid), || {
                children(id, DAEMON).contains(&pid)
            })
            .unwrap(),
        );
        daemon.pid = Some(pid);
        daemon
    }

    /// `program`, with this home's folders and no console.
    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .env("APPDATA", self.0.join("roaming"))
            .env("LOCALAPPDATA", self.0.join("local"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// Writes the daemon's config.
    fn config(&self, text: &str) {
        let dir = self.0.join(r"roaming\auto-clicker");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), text).unwrap();
    }

    fn log_path(&self) -> PathBuf {
        let path: PathBuf = ["local", "auto-clicker", "logs", "auto-clicker.log"]
            .iter()
            .collect();
        self.0.join(path)
    }

    /// The watchdog's log so far; empty until it has created it.
    fn watchdog_log(&self) -> String {
        std::fs::read_to_string(self.log_path().with_file_name("auto-clicker-watchdog.log"))
            .unwrap_or_default()
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
    fn running(&self, daemon: &mut Daemon) {
        let deadline = Instant::now() + START;
        while !self.log().contains(": running") {
            assert!(
                daemon.watchdog.try_wait().unwrap().is_none(),
                "{}",
                self.log()
            );
            assert!(Instant::now() < deadline, "not running: {}", self.log());
            sleep(Duration::from_millis(50));
        }
    }
}

/// A daemon started by its watchdog, both ended and reaped however the
/// test ends, a failed assertion included: a daemon left running would hold
/// the session's single instance. The watchdog's exit code is its daemon's.
struct Daemon {
    watchdog: Child,
    /// The daemon's process id, once found.
    pid: Option<u32>,
    /// The daemon's process, held from when it was found: ended through
    /// this handle, never by an id another process may have by then.
    held: Option<desktop::Held>,
}

impl Daemon {
    /// The daemon's process id.
    fn id(&self) -> u32 {
        self.pid.unwrap()
    }

    fn process(&self) -> desktop::Process {
        desktop::Process(self.id())
    }
}

impl Drop for Daemon {
    /// The daemon first, through its handle: its watchdog then releases
    /// what it held and ends on its own. A watchdog killed first would
    /// leave the daemon's message box in front of the next test. A daemon
    /// not found yet goes with its watchdog's tree.
    fn drop(&mut self) {
        if self.held.is_none() && matches!(self.watchdog.try_wait(), Ok(None)) {
            let _ = kill(self.watchdog.id(), true);
        }
        drop(self.held.take());
        let deadline = Instant::now() + END;
        while matches!(self.watchdog.try_wait(), Ok(None)) && Instant::now() < deadline {
            sleep(Duration::from_millis(20));
        }
        let _ = self.watchdog.kill();
        let _ = self.watchdog.wait();
    }
}

/// Ends process `pid` at once, as Task Manager's "End task" does, and with
/// `tree` everything it started too.
fn kill(pid: u32, tree: bool) -> ExitStatus {
    let mut command = Command::new("taskkill");
    command.arg("/F");
    if tree {
        command.arg("/T");
    }
    command
        .args(["/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
}

/// A process the test started, ended and reaped should the test fail
/// before it ends on its own; one that ended is only reaped.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let deadline = Instant::now() + END;
        while matches!(self.0.try_wait(), Ok(None)) && Instant::now() < deadline {
            sleep(Duration::from_millis(20));
        }
    }
}

/// The processes running `program` (a path) that `pid` started. Both: a
/// process whose parent ended keeps the parent's id, which a later process
/// may be given.
fn children(pid: u32, program: &str) -> Vec<u32> {
    let program = program.replace('\'', "''");
    powershell(&format!(
        "Get-CimInstance Win32_Process -Filter 'ParentProcessId = {pid}' \
         | Where-Object {{ $_.ExecutablePath -eq '{program}' }} \
         | ForEach-Object {{ $_.ProcessId }}"
    ))
    .lines()
    .filter_map(|line| line.trim().parse().ok())
    .collect()
}

/// Whether process `pid` still runs.
fn alive(pid: u32) -> bool {
    powershell(&format!(
        "[bool](Get-Process -Id {pid} -ErrorAction SilentlyContinue)"
    ))
    .trim()
        == "True"
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
fn ask_to_quit(daemon: &Daemon) {
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
    // It says why in a message box, its only window; closing it ends it,
    // and its watchdog with it.
    close_message_box(second.id(), || {
        second.watchdog.try_wait().unwrap().is_none()
    });
    assert_eq!(ended(&mut second.watchdog, END).code(), Some(3));
    // The second wrote nothing: the log is the first one's alone.
    assert_eq!(said(&home.log()), said(&before));
    ask_to_quit(&first);
    assert!(ended(&mut first.watchdog, END).success(), "{}", home.log());
    assert_eq!(
        said(&home.log()),
        said(&before),
        "the first quit with something to say"
    );
}

/// Waits for process `pid`'s message box, its only window, and closes it;
/// `running` must hold meanwhile.
fn close_message_box(pid: u32, mut running: impl FnMut() -> bool) {
    let deadline = Instant::now() + START;
    loop {
        let title = powershell(&format!(
            "(Get-Process -Id {pid} -ErrorAction SilentlyContinue).MainWindowTitle"
        ));
        if title.trim() == "Auto Clicker" {
            break;
        }
        assert!(running(), "ended without a box");
        assert!(Instant::now() < deadline, "no message box");
        sleep(Duration::from_millis(100));
    }
    powershell(&format!("[void](Get-Process -Id {pid}).CloseMainWindow()"));
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
        // The watchdog too: it sleeps on the daemon's process.
        let before = both(&daemon);
        sleep(IDLE);
        let after = both(&daemon);
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
    assert!(ended(&mut daemon.watchdog, END).success(), "{}", home.log());
    assert!(
        quiet,
        "Windows woke the daemon, or a thread ended, in each of {IDLE_TRIES} minutes \
         (wakes, threads ended, messages): {interrupted:?}"
    );
}

/// The threads of the daemon and its watchdog, and their switches so far.
fn both(daemon: &Daemon) -> BTreeMap<u32, u64> {
    let mut threads = switches(daemon.id());
    threads.extend(switches(daemon.watchdog.id()));
    threads
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

/// The menu labels these tests choose, with the Swedish form the daemon
/// shows on a Swedish Windows (as `tray_text` has them).
const LABELS: &[(&str, &str)] = &[
    ("Start clicking", "Börja klicka"),
    ("Stop clicking", "Sluta klicka"),
    ("Record (movement only)", "Spela in (bara rörelser)"),
    ("Stop recording", "Sluta spela in"),
    ("Play", "Spela upp"),
    ("Stop playing", "Sluta spela upp"),
    (
        "Settings (not on Windows yet)",
        "Inställningar (finns inte på Windows än)",
    ),
    ("", ""),
    ("Quit", "Avsluta"),
];

/// The language the daemon shows its tray in, picked as it picks it.
fn language() -> Language {
    auto_clicker_backend_windows::display_languages().map_or(Language::English, |names| {
        Language::from_tags(names.iter().map(String::as_str))
    })
}

/// `english`, a menu label, as the daemon shows it in the user's language.
fn label(english: &'static str) -> &'static str {
    match language() {
        Language::English => english,
        Language::Swedish => LABELS
            .iter()
            .find(|(en, _)| *en == english)
            .map(|&(_, sv)| sv)
            .unwrap_or_else(|| panic!("no Swedish form for {english:?}")),
    }
}

/// Whether the popup's `text` opens with its title saying `english`, or
/// `swedish` on a Swedish Windows.
fn titled(text: &str, english: &str, swedish: &str) -> bool {
    match language() {
        Language::English => text.starts_with(&format!("Auto clicker: {english}")),
        Language::Swedish => text.starts_with(&format!("Autoklickare: {swedish}")),
    }
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
    // A hold of F13, which no program acts on, rather than clicks where the
    // pointer is.
    home.config(HOLD_F13_LATER);
    let mut daemon = home.start();
    home.running(&mut daemon);
    let pid = daemon.process();
    let icon = promoted_icon(pid);
    let idle = |text: &str| titled(text, "idle", "vilar");
    popup_once(pid, icon, idle);
    // The menu as on Linux, Settings off until the window comes.
    desktop::click(icon.centre(), desktop::Button::Left).unwrap();
    let menu = wait_for("menu", desktop::shown_menu);
    let lines: Vec<(String, bool)> = menu
        .lines()
        .into_iter()
        .map(|line| (line.label, line.greyed))
        .collect();
    let line = |english: &'static str, greyed| (label(english).to_owned(), greyed);
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
    choose(icon, None, label("Start clicking"));
    popup_once(pid, icon, |text| titled(text, "starts in", "börjar om"));
    choose(icon, None, label("Stop clicking"));
    popup_once(pid, icon, idle);
    // Started and left to run: it holds, until Stop clicking.
    choose(icon, None, label("Start clicking"));
    wait_for("F13 held", || desktop::is_down(VK_F13).then_some(()));
    choose(icon, None, label("Stop clicking"));
    wait_for("F13 released", || (!desktop::is_down(VK_F13)).then_some(()));
    popup_once(pid, icon, idle);
    // Record, then Stop recording.
    choose(icon, None, label("Record (movement only)"));
    popup_once(pid, icon, |text| titled(text, "record", "spelar in"));
    choose(icon, None, label("Stop recording"));
    popup_once(pid, icon, |text| !titled(text, "record", "spelar in"));
    // The recording in the Play menu, then Stop playing.
    choose(icon, Some(label("Play")), RECORDING_LINE);
    popup_once(pid, icon, |text| titled(text, "plays in", "spelar upp om"));
    choose(icon, None, label("Stop playing"));
    popup_once(pid, icon, idle);
    // Quit ends the daemon, and its icon goes with it.
    let owner = desktop::icon_owner(pid).unwrap();
    choose(icon, None, label("Quit"));
    assert!(ended(&mut daemon.watchdog, END).success(), "{}", home.log());
    assert_eq!(owner.area(), None, "the icon outlived the daemon");
}

/// F13's virtual key: a key no program acts on.
const VK_F13: u16 = 0x7c;
/// The side (back) button's virtual key.
const VK_XBUTTON1: u16 = 0x05;
/// A hold of F13 that starts at once.
const HOLD_F13: &str =
    "version = 1\ntray_start_delay = 0\n\n[clicker]\nmode = \"hold_key\"\nkey = \"f13\"\n";
/// A hold of F13 after the tray's usual countdown.
const HOLD_F13_LATER: &str = "version = 1\n\n[clicker]\nmode = \"hold_key\"\nkey = \"f13\"\n";
/// A hold of the back (side) button that starts at once.
const HOLD_SIDE: &str =
    "version = 1\ntray_start_delay = 0\n\n[clicker]\nmode = \"hold_button\"\nbutton = \"back\"\n";
/// How soon a release must follow the daemon's end: X03 measured half a
/// millisecond; a hold lasts for ever.
const RELEASED: Duration = Duration::from_secs(2);

/// A daemon of its own holding `key` (a virtual key) as `config` says,
/// started from its tray icon.
fn holding(what: &str, config: &str, key: u16) -> (Home, Daemon) {
    let home = Home::new(what);
    home.config(config);
    let mut daemon = home.start();
    home.running(&mut daemon);
    let icon = promoted_icon(daemon.process());
    choose(icon, None, label("Start clicking"));
    wait_for("the hold", || desktop::is_down(key).then_some(()));
    (home, daemon)
}

/// How long after `since` `key` read up; fails past [`RELEASED`].
fn released(key: u16, since: Instant) -> Duration {
    loop {
        let up = !desktop::is_down(key);
        // Read after the key: an upper bound, even when it read up at once.
        let elapsed = since.elapsed();
        assert!(elapsed < RELEASED, "not up within {RELEASED:?}");
        if up {
            return elapsed;
        }
        sleep(Duration::from_millis(1));
    }
}

#[test]
#[ignore = "holds a key and a button in the desktop and kills the daemon: run in the WN01 VM through `vm run`"]
fn killing_the_daemon_while_it_holds_a_key_or_a_button_releases_it() {
    for (what, config, key) in [
        ("kill-key", HOLD_F13, VK_F13),
        ("kill-button", HOLD_SIDE, VK_XBUTTON1),
    ] {
        let (home, mut daemon) = holding(what, config, key);
        let killing = Instant::now();
        assert!(kill(daemon.id(), false).success());
        println!(
            "{what}: released {:?} after the kill",
            released(key, killing)
        );
        // The watchdog ends with its daemon's code: killed, 1.
        assert_eq!(
            ended(&mut daemon.watchdog, END).code(),
            Some(1),
            "{}",
            home.log()
        );
    }
}

#[test]
#[ignore = "holds a key in the desktop and ends the daemon's process tree: run in the WN01 VM through `vm run`"]
fn ending_the_daemons_process_tree_while_it_holds_a_key_releases_it() {
    let (home, mut daemon) = holding("tree", HOLD_F13, VK_F13);
    let killing = Instant::now();
    assert!(kill(daemon.id(), true).success());
    println!(
        "released {:?} after the tree's end",
        released(VK_F13, killing)
    );
    assert!(
        ended(&mut daemon.watchdog, END).code().is_some(),
        "{}",
        home.log()
    );
}

#[test]
#[ignore = "holds a key in the desktop and quits: run in the WN01 VM through `vm run`"]
fn a_quit_releases_and_leaves_no_watchdog_behind() {
    let (home, mut daemon) = holding("quit", HOLD_F13, VK_F13);
    ask_to_quit(&daemon);
    assert!(ended(&mut daemon.watchdog, END).success(), "{}", home.log());
    assert!(!desktop::is_down(VK_F13));
    assert!(!alive(daemon.id()));
    // The release counted as delivered by the daemon, on the real desktop:
    // nothing was left owed for the watchdog.
    assert!(
        !home.log().contains("could not be counted delivered"),
        "{}",
        home.log()
    );
    assert!(
        !home.watchdog_log().contains("released"),
        "{}",
        home.watchdog_log()
    );
}

#[test]
#[ignore = "holds a key in the desktop and kills the watchdog: run in the WN01 VM through `vm run`"]
fn the_watchdogs_end_stops_a_hold_at_once_and_the_daemon_says_why() {
    let (home, mut daemon) = holding("watchdog-ends", HOLD_F13, VK_F13);
    let killing = Instant::now();
    daemon.watchdog.kill().unwrap();
    let _ = daemon.watchdog.wait();
    println!(
        "released {:?} after the watchdog's end",
        released(VK_F13, killing)
    );
    // The daemon let go of its name before its message: a new start is
    // not told it is already running.
    close_message_box(daemon.id(), || alive(daemon.id()));
    let deadline = Instant::now() + END;
    while alive(daemon.id()) {
        assert!(Instant::now() < deadline, "the daemon outlived its message");
        sleep(Duration::from_millis(50));
    }
    assert!(
        home.log().contains("the watchdog ended; quitting"),
        "{}",
        home.log()
    );
}

#[test]
#[ignore = "starts the daemon bare in the desktop: run in the WN01 VM through `vm run`"]
fn a_bare_start_launches_the_watchdog_and_exits() {
    let home = Home::new("bare");
    let mut bare = home.command(DAEMON).spawn().unwrap();
    assert!(ended(&mut bare, END).success());
    // The bare start's own watchdog, as its child: `bare` still holds the
    // ended process, so its id names no other.
    let launcher = bare.id();
    let watchdog = wait_for("the watchdog", || {
        children(launcher, WATCHDOG).first().copied()
    });
    // Ended however the test ends: a daemon left running would hold the
    // session's single instance. Each held only once it is still the
    // process found, so the id named it.
    let _watchdog_guard = desktop::Held::open_checked(desktop::Process(watchdog), || {
        children(launcher, WATCHDOG).contains(&watchdog)
    })
    .unwrap();
    // Until the daemon is held, a failure ends the watchdog's whole tree:
    // the watchdog is held, so its id still names it.
    let found = std::panic::catch_unwind(|| {
        let pid = wait_for("its daemon", || children(watchdog, DAEMON).first().copied());
        let held = desktop::Held::open_checked(desktop::Process(pid), || {
            children(watchdog, DAEMON).contains(&pid)
        })
        .unwrap();
        (pid, held)
    });
    let (pid, _daemon_guard) = found.unwrap_or_else(|panic| {
        let _ = kill(watchdog, true);
        std::panic::resume_unwind(panic)
    });
    wait_for("the daemon running", || {
        home.log().contains(": running").then_some(())
    });
    assert!(
        Command::new("taskkill")
            .args(["/PID", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + END;
    while alive(watchdog) {
        assert!(
            Instant::now() < deadline,
            "the watchdog outlived its daemon"
        );
        sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "must run elevated, in the desktop: the WN01 VM through `vm run --elevated`"]
fn an_elevated_watchdog_refuses_before_it_writes_or_starts_anything() {
    // The usual unelevated run of every device test passes it by.
    // Not knowing fails the test rather than passing it by.
    if !auto_clicker_backend_windows::is_elevated().unwrap() {
        println!("not elevated: run this test alone through `vm run --elevated`");
        return;
    }
    let home = Home::new("elevated");
    let mut watchdog = Reaped(home.command(WATCHDOG).spawn().unwrap());
    let pid = watchdog.0.id();
    close_message_box(pid, || watchdog.0.try_wait().unwrap().is_none());
    assert_eq!(ended(&mut watchdog.0, END).code(), Some(4));
    assert!(children(pid, DAEMON).is_empty());
    // No log of its own, and no daemon's.
    assert!(!home.log_path().parent().unwrap().exists());
}

#[test]
#[ignore = "shows the daemon's refusal in the desktop: run in the WN01 VM through `vm run`"]
fn arguments_it_does_not_take_are_refused_never_a_new_start() {
    for arguments in [&["--table", "4", "--watchdog", "8"][..], &["--quit"]] {
        let home = Home::new("refused");
        let mut daemon = Reaped(home.command(DAEMON).args(arguments).spawn().unwrap());
        let pid = daemon.0.id();
        close_message_box(pid, || daemon.0.try_wait().unwrap().is_none());
        assert_eq!(ended(&mut daemon.0, END).code(), Some(5), "{arguments:?}");
        assert!(children(daemon.0.id(), WATCHDOG).is_empty());
        // Nothing was started, nothing logged.
        assert!(!home.log_path().exists(), "{arguments:?}");
    }
}

#[test]
#[ignore = "starts the daemon in the desktop: run in the WN01 VM through `vm run`"]
fn the_daemon_is_per_monitor_dpi_aware() {
    let home = Home::new("dpi");
    let mut daemon = home.start();
    home.running(&mut daemon);
    assert_eq!(desktop::per_monitor_aware(daemon.process()), Some(true));
    // The question tells the two apart: the watchdog never sets its own.
    assert_eq!(
        desktop::per_monitor_aware(desktop::Process(daemon.watchdog.id())),
        Some(false)
    );
    ask_to_quit(&daemon);
    assert!(ended(&mut daemon.watchdog, END).success(), "{}", home.log());
}

// Hotkeys (WN08). Keys are sent by scan code, as a keyboard sends them:
// what both the hotkeys and raw input see.

/// Ctrl's, Shift's and F6 to F8's scan codes.
const CTRL: u16 = 0x1d;
const SHIFT: u16 = 0x2a;
const F6: u16 = 0x40;
const F7: u16 = 0x41;
const F8: u16 = 0x42;
/// The virtual keys of F6 and F7.
const VK_F6: u16 = 0x75;
const VK_F7: u16 = 0x76;

/// A recording holding F13 for 20 s.
const HELD_F13: &str = "2026-11-09_19-00-00.000.json";
const HOLDING_F13: &str = r#"{"version":1,"name":"Hold","created":"2026-11-09T19:00:00.000+01:00","duration_us":20000000,"events":[
[0,"press","KEY_F13"],
[20000000,"release","KEY_F13"]
]}"#;

fn key(scan: u16, down: bool) {
    desktop::key(scan, false, down).unwrap();
}

/// Lets go of these keys (by scan code) when dropped, so a test that fails
/// leaves nothing of its own down in the desktop.
struct LetGo(Vec<u16>);

impl Drop for LetGo {
    fn drop(&mut self) {
        let up: Vec<(u16, bool)> = self.0.iter().map(|&scan| (scan, false)).collect();
        let _ = desktop::keys(&up);
    }
}

/// Ctrl+Shift+`scan`, every key let go at once, as a quick hand does.
fn combination(scan: u16) {
    let _up = LetGo(vec![CTRL, SHIFT, scan]);
    for (scan, down) in [
        (CTRL, true),
        (SHIFT, true),
        (scan, true),
        (scan, false),
        (SHIFT, false),
        (CTRL, false),
    ] {
        key(scan, down);
    }
}

/// A window of the test's own, clicked so it is in front.
fn in_front(no_hotkeys: bool) -> desktop::Front {
    let front = desktop::Front::open(no_hotkeys).unwrap();
    sleep(REOPEN);
    desktop::click(front.area().centre(), desktop::Button::Left).unwrap();
    wait_for("the test window in front", || {
        front.in_front().then_some(())
    });
    front
}

/// A daemon of its own, with its hotkeys registered.
fn with_hotkeys(what: &str, config: &str) -> (Home, Daemon) {
    let home = Home::new(what);
    home.config(config);
    let recordings = home.0.join(r"local\auto-clicker\recordings");
    std::fs::create_dir_all(&recordings).unwrap();
    std::fs::write(recordings.join(HELD_F13), HOLDING_F13).unwrap();
    let mut daemon = home.start();
    home.running(&mut daemon);
    wait_for("the hotkeys", || {
        desktop::HeldHotkey::taken(VK_F6).then_some(())
    });
    (home, daemon)
}

/// Waits until F13 reads `down`, at most [`START`].
fn f13(down: bool) {
    wait_for("F13", || (desktop::is_down(VK_F13) == down).then_some(()));
}

/// F13 stays up for a second: nothing started again.
fn stays_up() {
    let since = Instant::now();
    while since.elapsed() < Duration::from_secs(1) {
        assert!(!desktop::is_down(VK_F13), "started again");
        sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "presses hotkeys in the desktop: run in the WN01 VM through `vm run`"]
fn each_hotkey_starts_and_stops_its_job_while_another_window_has_focus() {
    let (home, daemon) = with_hotkeys("hotkeys", HOLD_F13);
    let pid = daemon.process();
    let front = in_front(false);
    // Clicking starts only once Ctrl and Shift are both up.
    let _up = LetGo(vec![CTRL, SHIFT, F6]);
    key(CTRL, true);
    key(SHIFT, true);
    key(F6, true);
    key(F6, false);
    sleep(Duration::from_millis(500));
    assert!(
        !desktop::is_down(VK_F13),
        "started with Ctrl and Shift down"
    );
    key(SHIFT, false);
    sleep(Duration::from_millis(300));
    assert!(!desktop::is_down(VK_F13), "started with Ctrl down");
    key(CTRL, false);
    f13(true);
    combination(F6);
    released(VK_F13, Instant::now());
    stays_up();
    // Playback: the hold recording, the newest.
    combination(F8);
    f13(true);
    combination(F8);
    released(VK_F13, Instant::now());
    stays_up();
    assert!(front.in_front(), "the window lost the focus");
    // The recording hotkey reaches the controller too, which cannot
    // record on Windows before WN09 names recordings by the local time.
    let icon = promoted_icon(pid);
    combination(F7);
    popup_once(pid, icon, |text| {
        text.contains("Not recording: the clock could not be read")
            || text.contains("Spelar inte in: klockan kunde inte läsas")
    });
    drop(home);
}

#[test]
#[ignore = "presses hotkeys in the desktop, many times: run in the WN01 VM through `vm run`"]
fn a_run_stops_by_its_hotkey_behind_any_window_and_never_starts_again() {
    let (home, daemon) = with_hotkeys("stops", HOLD_F13);
    let pid = daemon.process();
    let icon = promoted_icon(pid);
    // A window that turns hotkeys off: at idle nothing reads the keyboard,
    // so the hotkey starts nothing.
    let front = in_front(true);
    combination(F6);
    stays_up();
    drop(front);
    for round in 0..10 {
        let no_hotkeys = round % 2 == 0;
        choose(icon, None, label("Start clicking"));
        f13(true);
        let front = in_front(no_hotkeys);
        let pressed = Instant::now();
        combination(F6);
        let took = released(VK_F13, pressed);
        println!("round {round} (hotkeys off: {no_hotkeys}): released after {took:?}");
        stays_up();
        drop(front);
    }
    drop(home);
}

#[test]
#[ignore = "holds a hotkey in the desktop: run in the WN01 VM through `vm run`"]
fn a_combination_another_program_holds_is_named_in_the_tray() {
    let held = desktop::HeldHotkey::hold(VK_F7).unwrap();
    let (home, daemon) = with_hotkeys("taken", HOLD_F13);
    let pid = daemon.process();
    let icon = promoted_icon(pid);
    let text = popup_once(pid, icon, |text| {
        text.contains("Ctrl+Shift+F7 does not work: another program uses it")
            || text.contains("Ctrl+Shift+F7 fungerar inte: ett annat program använder den")
    });
    println!("{text}");
    // The others work.
    combination(F6);
    f13(true);
    combination(F6);
    released(VK_F13, Instant::now());
    drop(held);
    drop(home);
}

/// A daemon started unelevated by a one-off task of Windows' scheduler,
/// from an elevated test, which an elevated daemon refuses to be; ended,
/// with its watchdog and the task, when dropped.
struct Unelevated {
    task: String,
    watchdog: Option<u32>,
    daemon: Option<u32>,
    /// This build's watchdog, as the process list names it, and those that
    /// ran before the task: any other is the task's, should finding it fail.
    listed: String,
    before: Vec<u32>,
    /// Handles on both, once found and checked: while they are held, the
    /// ids name these processes and no other.
    held: Vec<desktop::Held>,
}

/// The watchdogs running from `path`.
fn watchdogs_of(path: &str) -> Vec<u32> {
    try_watchdogs_of(path).unwrap()
}

/// The watchdogs running from `path`, or why they could not be listed:
/// for cleanup, which must not panic while a failed test unwinds.
fn try_watchdogs_of(path: &str) -> std::io::Result<Vec<u32>> {
    let path = path.replace('\'', "''");
    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"Name = 'auto-clicker-watchdog.exe'\" \
         | Where-Object {{ $_.ExecutablePath -eq '{path}' }} \
         | ForEach-Object {{ $_.ProcessId }}"
    );
    let listed = desktop::powershell(&script, Duration::from_secs(60))?;
    Ok(listed
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect())
}

/// Runs `script` for cleanup: a failure is ignored rather than a panic
/// while a failed test unwinds.
fn quietly(script: &str) {
    let _ = desktop::powershell(script, Duration::from_secs(60));
}

/// Ends process `pid` (and its tree when `tree`) for cleanup, never
/// panicking.
fn kill_quietly(pid: u32, tree: bool) {
    let tree = if tree { " /T" } else { "" };
    let _ = Command::new("cmd")
        .args(["/c", &format!("taskkill /F{tree} /PID {pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Whether process `pid` still runs, for cleanup: not when it cannot be
/// asked.
fn alive_quietly(pid: u32) -> bool {
    let script = format!("[bool](Get-Process -Id {pid} -ErrorAction SilentlyContinue)");
    desktop::powershell(&script, Duration::from_secs(60)).is_ok_and(|out| out.trim() == "True")
}

impl Unelevated {
    fn start(home: &Home) -> (Unelevated, desktop::Process) {
        let script = home.0.join("start.cmd");
        let folder = |name: &str| home.0.join(name).display().to_string();
        let watchdog = std::path::Path::new(WATCHDOG)
            .canonicalize()
            .unwrap()
            .display()
            .to_string();
        // `canonicalize` gives a `\\?\` path; the process list does not.
        let listed = watchdog.trim_start_matches(r"\\?\").to_owned();
        std::fs::write(
            &script,
            format!(
                "set \"APPDATA={}\"\r\nset \"LOCALAPPDATA={}\"\r\nstart \"\" \"{watchdog}\"\r\n",
                folder("roaming"),
                folder("local"),
            ),
        )
        .unwrap();
        let task = format!("auto-clicker-test-{}", std::process::id());
        // Inside PowerShell's single quotes, an apostrophe is written twice.
        let script = script.display().to_string().replace('\'', "''");
        // Only a watchdog of this build that was not there before is ours.
        let before = watchdogs_of(&listed);
        // From here on, whatever the task started is ended when dropped.
        let mut started = Unelevated {
            task: task.clone(),
            watchdog: None,
            daemon: None,
            listed: listed.clone(),
            before: before.clone(),
            held: Vec::new(),
        };
        powershell(&format!(
            "schtasks /create /tn '{task}' /tr '\"{script}\"' /sc once /st 23:59 /rl limited /it /f | Out-Null; schtasks /run /tn '{task}' | Out-Null; schtasks /change /tn '{task}' /disable | Out-Null"
        ));
        let watchdog = wait_for("the unelevated watchdog", || {
            let mut new = watchdogs_of(&listed);
            new.retain(|pid| !before.contains(pid));
            new.first().copied()
        });
        let held = desktop::Held::open_checked(desktop::Process(watchdog), || {
            watchdogs_of(&listed).contains(&watchdog)
        })
        .unwrap();
        started.held.push(held);
        started.watchdog = Some(watchdog);
        let daemon = wait_for("its daemon", || children(watchdog, DAEMON).first().copied());
        let held = desktop::Held::open_checked(desktop::Process(daemon), || {
            children(watchdog, DAEMON).contains(&daemon)
        })
        .unwrap();
        started.held.push(held);
        started.daemon = Some(daemon);
        (started, desktop::Process(daemon))
    }
}

impl Drop for Unelevated {
    /// Ends the daemon first, so its watchdog releases whatever it held,
    /// then the watchdog should it still run. The handles, dropped last,
    /// end whatever is left.
    fn drop(&mut self) {
        if let Some(daemon) = self.daemon {
            kill_quietly(daemon, false);
        }
        let task = &self.task;
        // A launch still going would start its watchdog after the look.
        quietly(&format!("schtasks /end /tn '{task}' 2>&1 | Out-Null"));
        let watchdogs = match self.watchdog {
            Some(watchdog) => vec![watchdog],
            None => {
                let mut new = try_watchdogs_of(&self.listed).unwrap_or_default();
                new.retain(|pid| !self.before.contains(pid));
                // Held before they are ended, so each id still names the
                // watchdog found; one that fails the check is left alone.
                new.retain(|&pid| {
                    desktop::Held::open_checked(desktop::Process(pid), || {
                        try_watchdogs_of(&self.listed).is_ok_and(|now| now.contains(&pid))
                    })
                    .map(|held| self.held.push(held))
                    .is_ok()
                });
                new
            }
        };
        for watchdog in watchdogs {
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.daemon.is_some() && alive_quietly(watchdog) && Instant::now() < deadline {
                sleep(Duration::from_millis(100));
            }
            kill_quietly(watchdog, true);
        }
        quietly(&format!("schtasks /delete /tn '{task}' /f | Out-Null"));
    }
}

#[test]
#[ignore = "must run elevated, in the desktop: the WN01 VM through `vm run --elevated`"]
fn a_hotkey_start_with_a_program_running_as_administrator_in_front_is_refused() {
    // The usual unelevated run of every device test passes it by.
    if !auto_clicker_backend_windows::is_elevated().unwrap() {
        println!("not elevated: run this test alone through `vm run --elevated`");
        return;
    }
    let home = Home::new("elevated-front");
    home.config(HOLD_F13);
    let (daemon, pid) = Unelevated::start(&home);
    wait_for("the hotkeys", || {
        desktop::HeldHotkey::taken(VK_F6).then_some(())
    });
    // This test's own window, elevated as the test is, in front.
    let front = in_front(false);
    combination(F6);
    sleep(Duration::from_secs(1));
    assert!(!desktop::is_down(VK_F13), "clicking started");
    assert!(front.in_front());
    let icon = promoted_icon(pid);
    let text = popup_once(pid, icon, |text| {
        text.contains(
            "Not started: Auto Clicker can't click into a program running as administrator",
        ) || text.contains(
            "Startade inte: Auto Clicker kan inte klicka i ett program som körs som administratör",
        )
    });
    println!("{text}");
    drop(daemon);
    drop(home);
}
