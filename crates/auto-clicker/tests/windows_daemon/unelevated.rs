//! A daemon started unelevated from an elevated test, through Windows'
//! scheduler, for the tests that need an elevated window in front.

use super::*;

/// A daemon started unelevated by a one-off task of Windows' scheduler,
/// from an elevated test, which an elevated daemon refuses to be; ended,
/// with its watchdog and the task, when dropped.
pub(crate) struct Unelevated {
    task: String,
    pub(crate) watchdog: Option<u32>,
    pub(crate) daemon: Option<u32>,
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
    let mut command = Command::new("taskkill");
    command.arg("/F");
    if tree {
        command.arg("/T");
    }
    let spawned = command
        .args(["/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    // At most ten seconds: the held handles end what it leaves.
    if let Ok(mut child) = spawned {
        let deadline = Instant::now() + Duration::from_secs(10);
        while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
            sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Waits up to `within` for process `pid` to end, for cleanup.
pub(crate) fn wait_for_end(pid: u32, within: Duration) {
    let deadline = Instant::now() + within;
    while alive_quietly(pid) && Instant::now() < deadline {
        sleep(Duration::from_millis(100));
    }
}

/// Whether process `pid` still runs, for cleanup: taken as running when it
/// cannot be asked, so a watchdog keeps its time to release.
fn alive_quietly(pid: u32) -> bool {
    let script = format!("[bool](Get-Process -Id {pid} -ErrorAction SilentlyContinue)");
    desktop::powershell(&script, Duration::from_secs(60)).map_or(true, |out| out.trim() == "True")
}

impl Unelevated {
    /// Starts `home`'s watchdog unelevated, through a task named after
    /// `what`, and finds it and its daemon.
    pub(crate) fn start(home: &Home, what: &str) -> (Unelevated, desktop::Process) {
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
        let task = format!("auto-clicker-test-{}-{what}", std::process::id());
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
            // Its daemon ended: up to five seconds to release what it held.
            if self.daemon.is_some() {
                wait_for_end(watchdog, Duration::from_secs(5));
            }
            kill_quietly(watchdog, true);
        }
        quietly(&format!("schtasks /delete /tn '{task}' /f | Out-Null"));
    }
}
