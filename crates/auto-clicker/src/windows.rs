//! The daemon on Windows, before it can click (M5; WN05 replaces [`run`]).
//!
//! It refuses to run elevated (map D13): its input would reach programs
//! running as administrator, past the barrier Windows keeps between them and
//! the user's other programs, and it would read its config, recordings and
//! binary, which any of the user's programs can change, with an
//! administrator's rights. The check comes first, and a refusing daemon
//! writes nothing to the user's folders: it says why in a message box.

use std::io;
use std::process::ExitCode;

use auto_clicker_backend_windows::{display_languages, is_elevated, show_error};
use auto_clicker_core::dirs::log_path;
use auto_clicker_core::locale::Language;
use auto_clicker_core::log;

/// The daemon refused to run elevated, or could not tell whether it was.
const EXIT_ELEVATED: u8 = 4;

/// Runs the daemon to its end.
pub fn main() -> ExitCode {
    match refusal(is_elevated()) {
        Some(refusal) => {
            show_error("Auto Clicker", &refusal.text(language()));
            ExitCode::from(EXIT_ELEVATED)
        }
        None => run(),
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

/// Why the daemon does not start.
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

/// What the daemon does once it may run: for now, it says in its log that
/// Windows is not supported yet.
fn run() -> ExitCode {
    open_log();
    log::line(format_args!(
        "auto-clicker: Windows is not supported yet; this build does nothing"
    ));
    ExitCode::FAILURE
}

/// Sends the log to its file, since the daemon has no console. When it
/// cannot, lines go to standard error, which nobody sees, and that is all.
fn open_log() {
    let Some(path) = log_path() else {
        log::line(format_args!(
            "auto-clicker: no log file: LOCALAPPDATA is not set to an absolute folder"
        ));
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
    use super::*;

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
