//! The watchdog on Windows.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use auto_clicker_backend_windows::show_error;
use auto_clicker_backend_windows::watchdog::{
    DAEMON, Owed, Released, Section, Stayed, arguments, release, start_daemon,
};
use auto_clicker_core::dirs::log_path;
use auto_clicker_core::locale::Language;
use auto_clicker_core::log;

use crate::elevation::{self, language};

/// The message box's title.
const TITLE: &str = "Auto Clicker";
/// The watchdog's own log, beside the daemon's.
const LOG: &str = "auto-clicker-watchdog.log";
/// The watchdog could not start its daemon.
const EXIT_NOT_STARTED: u8 = 1;

pub fn main() -> ExitCode {
    // As the daemon does: elevated, the watchdog would write the user's
    // log folder and start the program beside it with administrator rights.
    if let Some(code) = elevation::refused() {
        return code;
    }
    open_log();
    let section = match Section::create() {
        Ok(section) => section,
        Err(err) => return not_started(&format!("the shared table could not be made: {err}")),
    };
    let daemon = match daemon_path().and_then(|path| start_daemon(&path, &section, arguments)) {
        Ok(daemon) => daemon,
        Err(err) => return not_started(&format!("{DAEMON} could not be started: {err}")),
    };
    // Nothing is logged from the daemon's start until its releases are
    // sent: a log write that stalls must not hold them up.
    let ended = daemon.wait();
    // Read once: from here on nothing writes it, and nothing in it is
    // trusted.
    let owed = Owed::parse(&section.table().copy());
    let released = release(&owed);
    let code = log_end(daemon.id(), ended);
    log_released(&owed, &released);
    // What may not have arrived is sent again until it does, or the
    // session ends (WN06b); a new daemon refuses starts meanwhile.
    let missed = released.missed();
    if let Some(line) = stayed_line(&released.send_until_delivered(), missed) {
        log::line(format_args!("auto-clicker-watchdog: {line}"));
    }
    // The daemon's own exit code, whole, so whoever started the watchdog
    // learns how the daemon ended, a crash's status included.
    std::process::exit(code.cast_signed())
}

/// Logs how the daemon `pid` ended, and gives the exit code to end with: the
/// daemon's, or 1 when the wait itself failed (and what the table held was
/// released at once: the daemon could no longer be watched).
fn log_end(pid: u32, ended: io::Result<u32>) -> u32 {
    match ended {
        Ok(code) => {
            log::line(format_args!(
                "auto-clicker-watchdog: the daemon (process {pid}) ended with {code:#x}"
            ));
            code
        }
        Err(err) => {
            log::line(format_args!(
                "auto-clicker-watchdog: the daemon (process {pid}) can no longer be waited on: {err}"
            ));
            1
        }
    }
}

/// Logs how many releases the table owed and how many of those may not
/// have arrived; never which keys.
fn log_released(owed: &Owed, released: &Released) {
    if owed.rejected > 0 {
        log::line(format_args!(
            "auto-clicker-watchdog: {} entries of the table named no key or button",
            owed.rejected
        ));
    }
    if released.owed == 0 {
        return;
    }
    let missed = released.missed();
    let again = if missed > 0 {
        "; sending them again once Windows lets them arrive"
    } else {
        ""
    };
    log::line(format_args!(
        "auto-clicker-watchdog: released {} held, {missed} of them maybe not delivered{again}",
        released.owed
    ));
}

/// What the log says of the watchdog's stay for `missed` releases; nothing
/// when there was none.
fn stayed_line(stayed: &Stayed, missed: usize) -> Option<String> {
    match stayed {
        Stayed::Delivered if missed == 0 => None,
        Stayed::Delivered => Some(format!("the {missed} releases left are delivered")),
        Stayed::SessionEnds(left) => {
            Some(format!("the session ends with {left} releases still owed"))
        }
        Stayed::Failed(err, left) => {
            Some(format!("cannot wait to send {left} releases again: {err}"))
        }
    }
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

/// Sends the log to its own file beside the daemon's; when it cannot,
/// lines go nowhere anyone sees.
fn open_log() {
    let Some(path) = log_path().map(|path| path.with_file_name(LOG)) else {
        return;
    };
    let _ = log::to_file(&path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watchdog_ends_with_the_daemons_whole_code_or_1_when_it_lost_it() {
        assert_eq!(log_end(7, Ok(0)), 0);
        // A crash's status, past a byte.
        assert_eq!(log_end(7, Ok(0xC000_0005)), 0xC000_0005);
        assert_eq!(log_end(7, Err(io::Error::other("gone"))), 1);
    }

    #[test]
    fn a_stay_that_did_not_deliver_everything_says_how_many_are_left() {
        assert_eq!(stayed_line(&Stayed::Delivered, 0), None);
        assert_eq!(
            stayed_line(&Stayed::Delivered, 2).as_deref(),
            Some("the 2 releases left are delivered")
        );
        assert_eq!(
            stayed_line(&Stayed::SessionEnds(2), 3).as_deref(),
            Some("the session ends with 2 releases still owed")
        );
        let failed = Stayed::Failed(io::Error::other("no window"), 1);
        assert_eq!(
            stayed_line(&failed, 1).as_deref(),
            Some("cannot wait to send 1 releases again: no window")
        );
    }
}
