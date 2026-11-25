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

mod notices;
mod platform;

use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use auto_clicker_backend_windows::{
    ClaimError, Instance, JoinError, MessageThread, claim, display_languages, is_elevated,
    show_error,
};
use auto_clicker_core::controller::Controller;
use auto_clicker_core::dirs::{NO_DATA_DIR, log_path};
use auto_clicker_core::locale::Language;
use auto_clicker_core::log;
use auto_clicker_core::recording::CurrentFile;
use tokio::sync::watch;

use crate::config::load_config;
use notices::{ENDED, Mark, Notices};
use platform::Windows;

/// The message box's title.
const TITLE: &str = "Auto Clicker";
/// The name the running daemon holds in its session, as on Linux's bus.
const INSTANCE_NAME: &str = "io.github.tobylo.AutoClicker";
/// Another daemon runs in this session.
const EXIT_ALREADY_RUNNING: u8 = 3;
/// The daemon refused to run elevated, or could not tell whether it was.
const EXIT_ELEVATED: u8 = 4;
/// How long the message thread may take to create its window.
const MESSAGE_THREAD_START: Duration = Duration::from_secs(5);
/// How long the message thread may take to end once told to.
const MESSAGE_THREAD_JOIN: Duration = Duration::from_secs(1);

/// Runs the daemon to its end.
pub fn main() -> ExitCode {
    if let Some(refusal) = refusal(is_elevated()) {
        show_error(TITLE, &refusal.text(language()));
        return ExitCode::from(EXIT_ELEVATED);
    }
    // Claimed before anything else: a second daemon reads no config and
    // writes no log.
    let instance = match claim(INSTANCE_NAME) {
        Ok(instance) => instance,
        Err(err) => return not_alone(&err),
    };
    open_log();
    let status = run();
    drop::<Instance>(instance);
    status
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

/// Why the daemon does not start, given whether it runs elevated; none when
/// it may. Not knowing refuses too.
fn refusal(elevated: io::Result<bool>) -> Option<Refusal> {
    match elevated {
        Ok(false) => None,
        Ok(true) => Some(Refusal::Elevated),
        Err(err) => Some(Refusal::Unknown(err)),
    }
}

/// Why an elevated daemon does not start.
#[derive(Debug)]
enum Refusal {
    /// It runs elevated.
    Elevated,
    /// Windows could not say whether it does.
    Unknown(io::Error),
}

impl Refusal {
    /// What the message box says, in `language`.
    fn text(&self, language: Language) -> String {
        let why = match (self, language) {
            (Refusal::Elevated, Language::English) => {
                "Auto Clicker does not run as administrator.".to_owned()
            }
            (Refusal::Elevated, Language::Swedish) => {
                "Auto Clicker körs inte som administratör.".to_owned()
            }
            (Refusal::Unknown(err), Language::English) => format!(
                "Auto Clicker could not tell whether it runs as administrator ({err}), so it does not start."
            ),
            (Refusal::Unknown(err), Language::Swedish) => format!(
                "Auto Clicker kunde inte avgöra om det körs som administratör ({err}), så det startar inte."
            ),
        };
        format!("{why}\n\n{}", reason(language))
    }
}

/// Why the daemon never runs as administrator, and what to do instead.
fn reason(language: Language) -> &'static str {
    match language {
        Language::English => {
            "As administrator, the input it sends would reach other programs running as \
             administrator, and it would read its settings and recordings, which any of your \
             programs can change, with an administrator's rights.\n\n\
             Start it without \"Run as administrator\". With User Account Control turned off, \
             every program runs as administrator, and Auto Clicker cannot run."
        }
        Language::Swedish => {
            "Som administratör skulle indata som programmet skickar nå andra program som körs \
             som administratör, och det skulle läsa sina inställningar och inspelningar, som \
             alla dina program kan ändra, med administratörens rättigheter.\n\n\
             Starta det utan \"Kör som administratör\". Med Kontroll av användarkonto (UAC) \
             avstängd körs alla program som administratör, och då kan Auto Clicker inte köras."
        }
    }
}

/// The language the user reads Windows in; English when Windows cannot
/// say, since a refusing daemon has nowhere to log that.
fn language() -> Language {
    display_languages().map_or(Language::English, |names| {
        Language::from_tags(names.iter().map(String::as_str))
    })
}

/// Runs the controller beside the message thread until Quit, logoff or
/// the message thread's end; then ends the message thread.
fn run() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => return fail(&format!("could not start the runtime: {err}")),
    };
    let (config, config_error, file) = load_config();
    let (mut controller, links) = Controller::new(Windows::new(), None, config, config_error);
    controller.keep_config_file(file);
    controller.keep_current_file(CurrentFile::of_environment());
    let released = Arc::new(Mark::default());
    let mut notices = Notices::new(links.send.clone(), Arc::clone(&released));
    let thread =
        match MessageThread::spawn(move |event| notices.handle(event), MESSAGE_THREAD_START) {
            Ok(thread) => thread,
            Err(err) => return fail(&format!("could not start the message thread: {err}")),
        };
    log::line(format_args!(
        "auto-clicker {}: running",
        env!("CARGO_PKG_VERSION")
    ));
    // Nothing shows the view until the tray arrives (WN07); it is kept so
    // the controller always has a reader.
    let _view = links.view;
    runtime.block_on(async {
        tokio::spawn(report_releases(
            links.released_for_sleep,
            Arc::clone(&released),
        ));
        controller.run().await;
    });
    // The controller has ended and released everything: a logoff or a
    // sleep waiting for that may go on, whether or not its report was
    // passed on before the runtime stopped.
    released.raise(ENDED);
    end(thread)
}

/// Passes the controller's reports of letting go before sleep to the
/// message thread, which waits for them, until the controller is gone.
async fn report_releases(mut reports: watch::Receiver<u64>, released: Arc<Mark>) {
    while reports.changed().await.is_ok() {
        released.raise(*reports.borrow_and_update());
    }
}

/// Ends the message thread within its deadline; its window, and with it
/// every registration (and from WN07 the tray icon), goes first.
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
    fn only_a_process_known_not_to_be_elevated_starts() {
        assert!(refusal(Ok(false)).is_none());
        assert!(matches!(refusal(Ok(true)), Some(Refusal::Elevated)));
        let unknown = refusal(Err(io::Error::from_raw_os_error(5)));
        assert!(matches!(unknown, Some(Refusal::Unknown(_))));
    }

    #[test]
    fn the_refusal_says_why_in_the_users_language() {
        let elevated = Refusal::Elevated.text(Language::English);
        assert!(elevated.starts_with("Auto Clicker does not run as administrator."));
        assert!(elevated.contains("User Account Control turned off"));
        let swedish = Refusal::Elevated.text(Language::Swedish);
        assert!(swedish.starts_with("Auto Clicker körs inte som administratör."));
        assert!(swedish.contains("Kontroll av användarkonto"));
        let unknown = Refusal::Unknown(io::Error::other("no token")).text(Language::English);
        assert!(unknown.contains("(no token)"), "{unknown}");
        assert!(unknown.contains("Run as administrator"));
        let okänt = Refusal::Unknown(io::Error::other("no token")).text(Language::Swedish);
        assert!(okänt.contains("(no token)") && okänt.contains("Kör som administratör"));
    }
}
