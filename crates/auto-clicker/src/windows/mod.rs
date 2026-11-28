//! The daemon on Windows (M5): one per session, the controller on tokio's
//! single-thread runtime, and beside it the Win32 message thread, asleep in
//! `GetMessage` until Windows says something (brief decision 4, map D11).
//!
//! It refuses to run elevated (map D13): its input would reach programs
//! running as administrator, past the barrier Windows keeps between them and
//! the user's other programs, and it would read its config, recordings and
//! binary, which any of the user's programs can change, with an
//! administrator's rights. The check comes first, and a refusing daemon
//! writes nothing to the user's folders: it says why in a message box. A
//! second daemon in the same session says so the same way, before it reads
//! or writes anything.
//!
//! It runs as the child of its watchdog (M5 brief decision 6), which
//! releases what it held should it die. Started bare, it starts the
//! watchdog, which starts it again with the two handles it needs; started
//! with anything else, it refuses, so a bad start cannot loop. Should the
//! watchdog end, the daemon stops, releases and quits, saying why.

pub mod elevation;
mod notices;
mod platform;
mod tray;
mod window;

use std::io;
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use auto_clicker_backend_windows::watchdog::{
    self as launch, Attached, Earlier, EarlierWait, Launch, WATCHDOG, WatchdogWait, attach,
    earlier_watchdog, wait_for_watchdog,
};
use auto_clicker_backend_windows::{
    ClaimError, Instance, JoinError, MessageThread, Remote, become_per_monitor_aware, claim,
    show_error,
};
use auto_clicker_core::controller::Controller;
use auto_clicker_core::dirs::{NO_DATA_DIR, log_path};
use auto_clicker_core::locale::Language;
use auto_clicker_core::log;
use auto_clicker_core::recording::CurrentFile;
use tokio::sync::watch;

use crate::config::load_config;
use elevation::language;
use notices::{ENDED, Mark, Notices, on_watchdog_end};
use platform::Windows;

/// The message box's title.
const TITLE: &str = "Auto Clicker";
/// The name the running daemon holds in its session, as on Linux's bus.
const INSTANCE_NAME: &str = "io.github.tobylo.AutoClicker";
/// Another daemon runs in this session.
const EXIT_ALREADY_RUNNING: u8 = 3;
/// The daemon was started with arguments or handles it does not take, or
/// started bare and could not start its watchdog.
const EXIT_LAUNCH: u8 = 5;
/// The daemon's watchdog ended, so it stopped.
const EXIT_WATCHDOG_ENDED: u8 = 6;
/// How long the wait on the watchdog, or on an earlier one, may take to
/// end once stopped.
const WATCHDOG_WAIT_STOP: Duration = Duration::from_secs(1);
/// How long the message thread may take to create its window.
const MESSAGE_THREAD_START: Duration = Duration::from_secs(5);
/// How long the message thread may take to end once told to.
const MESSAGE_THREAD_JOIN: Duration = Duration::from_secs(1);

/// Runs the daemon to its end.
pub fn main() -> ExitCode {
    // Before any window exists, which would fix the awareness; every size
    // the daemon reads is then in physical pixels.
    let aware = become_per_monitor_aware();
    if let Some(code) = elevation::refused() {
        return code;
    }
    let attached = match launch::parse(std::env::args_os().skip(1)) {
        Ok(Launch::Bare) => return start_watchdog(),
        Ok(Launch::Watched(handles)) => match attach(handles) {
            Ok(attached) => attached,
            Err(err) => return refuse_launch(&LaunchRefusal::Handles(err)),
        },
        Err(why) => return refuse_launch(&LaunchRefusal::Arguments(why)),
    };
    // Claimed before anything else: a second daemon reads no config and
    // writes no log.
    let instance = match claim(INSTANCE_NAME) {
        Ok(instance) => instance,
        Err(err) => return not_alone(&err),
    };
    open_log();
    if let Err(err) = aware {
        log::line(format_args!(
            "auto-clicker: not per-monitor DPI aware ({err}); absolute moves may land off"
        ));
    }
    let (status, watchdog_ended) = run(attached);
    // The name goes before the message, which waits for the user: a new
    // start meanwhile is not told the daemon is already running.
    drop::<Instance>(instance);
    if watchdog_ended {
        show_error(TITLE, watchdog_ended_text(language()));
        return ExitCode::from(EXIT_WATCHDOG_ENDED);
    }
    status
}

/// Starts the watchdog beside this program, which starts the daemon with
/// its handles, and exits. A watchdog that cannot start is said in a
/// message, never tried again.
fn start_watchdog() -> ExitCode {
    let started = std::env::current_exe()
        .map(|daemon| daemon.with_file_name(WATCHDOG))
        .and_then(|watchdog| Command::new(watchdog).spawn());
    match started {
        Ok(_) => ExitCode::SUCCESS,
        Err(err) => refuse_launch(&LaunchRefusal::NoWatchdog(err)),
    }
}

/// Why the daemon does not start as it was started.
#[derive(Debug)]
enum LaunchRefusal {
    /// Its arguments are not the ones it takes.
    Arguments(String),
    /// The handles they name are not what the watchdog gives.
    Handles(io::Error),
    /// Started bare, it could not start its watchdog.
    NoWatchdog(io::Error),
}

impl LaunchRefusal {
    /// What the message box says, in `language`.
    fn text(&self, language: Language) -> String {
        match (self, language) {
            (LaunchRefusal::Arguments(why), Language::English) => format!(
                "Auto Clicker was started with arguments it does not take ({why}), so it does \
                 not start. Start it without arguments."
            ),
            (LaunchRefusal::Arguments(why), Language::Swedish) => format!(
                "Auto Clicker startades med argument som det inte tar emot ({why}), så det \
                 startar inte. Starta det utan argument."
            ),
            (LaunchRefusal::Handles(err), Language::English) => format!(
                "Auto Clicker could not take over from its watchdog ({err}), so it does not \
                 start. Start it without arguments."
            ),
            (LaunchRefusal::Handles(err), Language::Swedish) => format!(
                "Auto Clicker kunde inte ta över från sin vakt ({err}), så det startar inte. \
                 Starta det utan argument."
            ),
            (LaunchRefusal::NoWatchdog(err), Language::English) => format!(
                "Auto Clicker could not start its watchdog, {WATCHDOG} ({err}), so it does \
                 not start. The watchdog lets go of held keys and buttons should Auto Clicker \
                 end unexpectedly; it belongs in the same folder."
            ),
            (LaunchRefusal::NoWatchdog(err), Language::Swedish) => format!(
                "Auto Clicker kunde inte starta sin vakt, {WATCHDOG} ({err}), så det startar \
                 inte. Vakten släpper nedtryckta tangenter och knappar om Auto Clicker skulle \
                 avslutas oväntat; den hör hemma i samma mapp."
            ),
        }
    }
}

/// Says why the daemon does not start as it was started, and exits.
fn refuse_launch(why: &LaunchRefusal) -> ExitCode {
    show_error(TITLE, &why.text(language()));
    ExitCode::from(EXIT_LAUNCH)
}

/// What the message box says once the watchdog ended and the daemon
/// stopped.
fn watchdog_ended_text(language: Language) -> &'static str {
    match language {
        Language::English => {
            "Auto Clicker stopped: its watchdog, which lets go of held keys and buttons should \
             Auto Clicker end unexpectedly, has ended.\n\n\
             To start it again, sign out and in again, or run auto-clicker.exe."
        }
        Language::Swedish => {
            "Auto Clicker har stoppats: dess vakt, som släpper nedtryckta tangenter och knappar \
             om Auto Clicker skulle avslutas oväntat, har avslutats.\n\n\
             Starta det igen genom att logga ut och in igen, eller kör auto-clicker.exe."
        }
    }
}

/// Says why the daemon does not start beside another, and the exit status.
fn not_alone(err: &ClaimError) -> ExitCode {
    let language = language();
    show_error(TITLE, &claim_text(err, language));
    match err {
        ClaimError::Taken => ExitCode::from(EXIT_ALREADY_RUNNING),
        ClaimError::Failed(_) => ExitCode::FAILURE,
    }
}

/// What the message box says when the name cannot be claimed.
fn claim_text(err: &ClaimError, language: Language) -> String {
    match (err, language) {
        (ClaimError::Taken, Language::English) => "Auto Clicker is already running.".to_owned(),
        (ClaimError::Taken, Language::Swedish) => "Auto Clicker körs redan.".to_owned(),
        (ClaimError::Failed(err), Language::English) => format!(
            "Auto Clicker could not check whether it is already running ({err}), so it does not start."
        ),
        (ClaimError::Failed(err), Language::Swedish) => {
            format!("Auto Clicker kunde inte se om det redan körs ({err}), så det startar inte.")
        }
    }
}

/// Runs the controller beside the message thread until Quit, logoff, the
/// watchdog's end or the message thread's end; then ends the message
/// thread. The status, and whether the watchdog ended.
fn run(attached: Attached) -> (ExitCode, bool) {
    // The I/O driver carries the settings window's pipes.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => return (fail(&format!("could not start the runtime: {err}")), false),
    };
    let Attached { table, watchdog } = attached;
    let (config, config_error, file) = load_config();
    let message_thread = Arc::new(OnceLock::new());
    let platform = Windows::new(table, Arc::clone(&message_thread));
    let (mut controller, links) = Controller::new(platform, None, config, config_error);
    controller.keep_config_file(file);
    controller.keep_current_file(CurrentFile::of_environment());
    let released = Arc::new(Mark::default());
    let mut notices = Notices::new(links.send.clone(), Arc::clone(&released));
    // Looked for before the thread runs, so no start is allowed before it.
    let earlier = look_for_earlier_watchdog();
    if earlier.is_some() {
        notices.earlier_watchdog();
    }
    notices.open_settings_with(keep_window(&runtime, controller.window_opener()));
    let commands = notices.commands();
    let thread =
        match MessageThread::spawn(move |event| notices.handle(event), MESSAGE_THREAD_START) {
            Ok(thread) => thread,
            Err(err) => {
                return (
                    fail(&format!("could not start the message thread: {err}")),
                    false,
                );
            }
        };
    // Before the controller runs, so its reader finds the thread.
    let _ = message_thread.set(thread.remote());
    let earlier = earlier.and_then(|earlier| wait_for_earlier(earlier, thread.remote()));
    // No run starts unwatched: the controller has not run yet.
    let watchdog_ended = Arc::new(AtomicBool::new(false));
    let told = on_watchdog_end(links.send.clone(), Arc::clone(&watchdog_ended));
    let wait = match wait_for_watchdog(watchdog, told) {
        Ok(wait) => wait,
        Err(err) => {
            let _ = end(thread);
            return (
                fail(&format!("could not wait on the watchdog: {err}")),
                false,
            );
        }
    };
    log::line(format_args!(
        "auto-clicker {}: running",
        env!("CARGO_PKG_VERSION")
    ));
    runtime.block_on(async {
        tokio::spawn(report_releases(
            links.released_for_sleep,
            Arc::clone(&released),
        ));
        tokio::spawn(tray::follow(
            links.view,
            thread.remote(),
            language(),
            commands,
        ));
        controller.run().await;
    });
    // The controller has ended and released everything: a logoff or a
    // sleep waiting for that may go on, whether or not its report was
    // passed on before the runtime stopped.
    released.raise(ENDED);
    stop_waits(wait, earlier);
    (end(thread), watchdog_ended.load(Ordering::Acquire))
}

/// Starts the settings window's keeper on `runtime`, which runs it once the
/// controller does; a choice of Settings goes to what this returns.
fn keep_window(
    runtime: &tokio::runtime::Runtime,
    opener: auto_clicker_core::controller::WindowOpener,
) -> tokio::sync::mpsc::UnboundedSender<()> {
    let _inside = runtime.enter();
    window::keep(opener)
}

/// Stops the waits on the watchdog and on an earlier one, each within its
/// deadline, and logs one that did not end in time.
fn stop_waits(watchdog: WatchdogWait, earlier: Option<EarlierWait>) {
    if !watchdog.stop(WATCHDOG_WAIT_STOP) {
        log::line(format_args!(
            "auto-clicker: the wait on the watchdog did not end in time"
        ));
    }
    if let Some(earlier) = earlier
        && !earlier.stop(WATCHDOG_WAIT_STOP)
    {
        log::line(format_args!(
            "auto-clicker: the wait on the earlier watchdog did not end in time"
        ));
    }
}

/// An earlier watchdog that still sends releases, if there is one: then
/// starts are refused until it is done. One that cannot be looked for
/// counts as none, so a broken name never refuses starts for good; its
/// worst outcome is a stray key-up.
fn look_for_earlier_watchdog() -> Option<Earlier> {
    match earlier_watchdog() {
        Ok(earlier) => earlier,
        Err(err) => {
            log::line(format_args!(
                "auto-clicker: cannot look for an earlier watchdog ({err}); starting as if none"
            ));
            None
        }
    }
}

/// Waits for `earlier` to be done, then tells the message thread through
/// `remote`, which allows starts again. A wait that cannot start, or that
/// fails, tells it at once.
fn wait_for_earlier(earlier: Earlier, remote: Remote) -> Option<EarlierWait> {
    let told = remote.clone();
    let waiting = earlier.wait(move |failed| {
        // Told before the log is written.
        let posted = told.earlier_watchdog_done();
        if let Some(err) = failed {
            log::line(format_args!(
                "auto-clicker: the earlier watchdog cannot be waited on ({err}); starting as if it were done"
            ));
        }
        if let Err(err) = posted {
            log::line(format_args!(
                "auto-clicker: the message thread could not be told the earlier watchdog is done: {err}"
            ));
        }
    });
    match waiting {
        Ok(wait) => Some(wait),
        Err(err) => {
            let _ = remote.earlier_watchdog_done();
            log::line(format_args!(
                "auto-clicker: cannot wait for the earlier watchdog ({err}); starting as if it were done"
            ));
            None
        }
    }
}

/// Passes the controller's reports of letting go before sleep to the
/// message thread, which waits for them, until the controller is gone.
async fn report_releases(mut reports: watch::Receiver<u64>, released: Arc<Mark>) {
    while reports.changed().await.is_ok() {
        released.raise(*reports.borrow_and_update());
    }
}

/// Ends the message thread within its deadline; its window, and with it
/// every registration and the tray icon, goes first.
fn end(thread: MessageThread) -> ExitCode {
    match thread.quit_and_join(MESSAGE_THREAD_JOIN) {
        Ok(()) => ExitCode::SUCCESS,
        Err(JoinError::TimedOut) => fail("the message thread did not end in time"),
        Err(JoinError::Panicked) => fail("the message thread panicked"),
        // Already logged when it ended.
        Err(JoinError::Failed(_)) => ExitCode::FAILURE,
    }
}

fn fail(message: &str) -> ExitCode {
    log::line(format_args!("auto-clicker: {message}"));
    ExitCode::FAILURE
}

/// Sends the log to its file, since the daemon has no console. When it
/// cannot, lines go to standard error, which nobody sees, and that is all.
fn open_log() {
    let Some(path) = log_path() else {
        log::line(format_args!("auto-clicker: no log file: {NO_DATA_DIR}"));
        return;
    };
    if let Err(err) = log::to_file(&path) {
        log::line(format_args!(
            "auto-clicker: no log file at {}: {err}",
            path.display()
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Set in the child process the next test starts.
    const CHILD: &str = "AUTO_CLICKER_TEST_CHILD";

    #[test]
    fn the_daemon_logs_to_its_file_under_localappdata() {
        if std::env::var_os(CHILD).is_some() {
            open_log();
            log::line(format_args!("auto-clicker: a line"));
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "auto-clicker-test-{}-{:016x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "windows::tests::the_daemon_logs_to_its_file_under_localappdata",
                "--exact",
            ])
            .env(CHILD, "1")
            .env("LOCALAPPDATA", &dir)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        let log: PathBuf = ["auto-clicker", "logs", "auto-clicker.log"]
            .iter()
            .collect();
        let text = std::fs::read_to_string(dir.join(log)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(status.success());
        assert_eq!(text, "auto-clicker: a line\n");
    }

    #[test]
    fn the_controllers_release_reports_reach_the_message_threads_mark() {
        let (report, reports) = watch::channel(0);
        let released = Arc::new(Mark::default());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let passing = tokio::spawn(report_releases(reports, Arc::clone(&released)));
            report.send_replace(2);
            // The controller is gone: the task passes on the last report
            // and ends.
            drop(report);
            passing.await.unwrap();
        });
        assert!(released.wait_for(2, Duration::ZERO));
    }

    #[test]
    fn a_second_daemon_says_why_in_the_users_language() {
        assert_eq!(
            claim_text(&ClaimError::Taken, Language::English),
            "Auto Clicker is already running."
        );
        assert_eq!(
            claim_text(&ClaimError::Taken, Language::Swedish),
            "Auto Clicker körs redan."
        );
        let failed = ClaimError::Failed(io::Error::other("no name"));
        let english = claim_text(&failed, Language::English);
        assert!(english.contains("(no name)") && english.contains("does not start"));
        let svenska = claim_text(&failed, Language::Swedish);
        assert!(svenska.contains("(no name)") && svenska.contains("startar inte"));
    }

    #[test]
    fn a_refused_launch_says_why_in_the_users_language() {
        let arguments = LaunchRefusal::Arguments("got --quit".to_owned());
        let english = arguments.text(Language::English);
        assert!(english.contains("(got --quit)") && english.contains("without arguments"));
        let svenska = arguments.text(Language::Swedish);
        assert!(svenska.contains("(got --quit)") && svenska.contains("utan argument"));
        let handles = LaunchRefusal::Handles(io::Error::other("no section"));
        assert!(
            handles
                .text(Language::English)
                .contains("take over from its watchdog (no section)")
        );
        assert!(
            handles
                .text(Language::Swedish)
                .contains("sin vakt (no section)")
        );
        let missing = LaunchRefusal::NoWatchdog(io::Error::other("not found"));
        let english = missing.text(Language::English);
        assert!(
            english.contains(WATCHDOG) && english.contains("(not found)"),
            "{english}"
        );
        let svenska = missing.text(Language::Swedish);
        assert!(
            svenska.contains(WATCHDOG) && svenska.contains("samma mapp"),
            "{svenska}"
        );
    }

    #[test]
    fn a_daemon_whose_watchdog_ended_says_how_to_start_again() {
        assert!(watchdog_ended_text(Language::English).contains("run auto-clicker.exe"));
        assert!(watchdog_ended_text(Language::Swedish).contains("kör auto-clicker.exe"));
    }
}
