//! The watchdog on Windows.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use auto_clicker_backend_windows::watchdog::{
    DAEMON, Daemon, Owed, Section, arguments, release, start_daemon,
};
use auto_clicker_backend_windows::{Delivery, display_languages, show_error};
use auto_clicker_core::dirs::log_path;
use auto_clicker_core::locale::Language;
use auto_clicker_core::log;

/// The message box's title.
const TITLE: &str = "Auto Clicker";
/// The watchdog's own log, beside the daemon's.
const LOG: &str = "auto-clicker-watchdog.log";
/// The watchdog could not start its daemon.
const EXIT_NOT_STARTED: u8 = 1;

pub fn main() -> ExitCode {
    open_log();
    let section = match Section::create() {
        Ok(section) => section,
        Err(err) => return not_started(&format!("the shared table could not be made: {err}")),
    };
    let daemon = match daemon_path().and_then(|path| start_daemon(&path, &section, arguments)) {
        Ok(daemon) => daemon,
        Err(err) => return not_started(&format!("{DAEMON} could not be started: {err}")),
    };
    log::line(format_args!(
        "auto-clicker-watchdog: started the daemon (process {})",
        daemon.id()
    ));
    let code = ended(&daemon);
    // Read once: from here on nothing writes it, and nothing in it is
    // trusted.
    let owed = Owed::parse(&section.table().copy());
    release_owed(&owed);
    // The daemon's own exit code, so whoever started the watchdog learns how
    // the daemon ended; one that does not fit is a failure.
    u8::try_from(code).map_or(ExitCode::FAILURE, ExitCode::from)
}

/// Waits for the daemon to end; its exit code, or a failure's when the wait
/// itself failed (and then what the table holds is released at once: the
/// daemon can no longer be watched).
fn ended(daemon: &Daemon) -> u32 {
    match daemon.wait() {
        Ok(code) => {
            log::line(format_args!(
                "auto-clicker-watchdog: the daemon ended with {code}"
            ));
            code
        }
        Err(err) => {
            log::line(format_args!(
                "auto-clicker-watchdog: the daemon can no longer be waited on: {err}"
            ));
            1
        }
    }
}

/// Sends each release the table owes, once, and logs how many went and how
/// many may not have arrived; never which keys.
fn release_owed(owed: &Owed) {
    if owed.rejected > 0 {
        log::line(format_args!(
            "auto-clicker-watchdog: {} entries of the table named no key or button",
            owed.rejected
        ));
    }
    if owed.is_empty() {
        return;
    }
    let sent = release(owed);
    let missed = sent
        .iter()
        .filter(|(_, delivery)| *delivery != Delivery::Delivered)
        .count();
    log::line(format_args!(
        "auto-clicker-watchdog: released {} held, {missed} of them maybe not delivered",
        sent.len()
    ));
}

/// The daemon's program, beside this one.
fn daemon_path() -> io::Result<PathBuf> {
    Ok(std::env::current_exe()?.with_file_name(DAEMON))
}

/// Says why the daemon was not started, and exits.
fn not_started(why: &str) -> ExitCode {
    log::line(format_args!("auto-clicker-watchdog: {why}"));
    let text = match language() {
        Language::English => format!("Auto Clicker could not start: {why}."),
        Language::Swedish => format!("Auto Clicker kunde inte starta: {why}."),
    };
    show_error(TITLE, &text);
    ExitCode::from(EXIT_NOT_STARTED)
}

/// The language the user reads Windows in; English when Windows cannot say.
fn language() -> Language {
    display_languages().map_or(Language::English, |names| {
        Language::from_tags(names.iter().map(String::as_str))
    })
}

/// Sends the log to its own file beside the daemon's; when it cannot,
/// lines go nowhere anyone sees.
fn open_log() {
    let Some(path) = log_path().map(|path| path.with_file_name(LOG)) else {
        return;
    };
    let _ = log::to_file(&path);
}
