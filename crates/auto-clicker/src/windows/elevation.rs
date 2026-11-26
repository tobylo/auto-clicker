//! Why a program of Auto Clicker's does not run elevated, and what it says
//! instead. Shared by the daemon and its watchdog, which both check before
//! they read or write anything of the user's.

use std::io;
use std::process::ExitCode;

use auto_clicker_backend_windows::{display_languages, is_elevated, show_error};
use auto_clicker_core::locale::Language;

/// The message box's title.
const TITLE: &str = "Auto Clicker";
/// The program refused to run elevated, or could not tell whether it was.
const EXIT_ELEVATED: u8 = 4;

/// Refuses to go on when this process runs elevated, or cannot tell: says
/// why in a message box, and gives the exit code. Called before anything
/// of the user's is read or written.
pub fn refused() -> Option<ExitCode> {
    let refusal = refusal(is_elevated())?;
    show_error(TITLE, &refusal.text(language()));
    Some(ExitCode::from(EXIT_ELEVATED))
}

/// Why the program does not start, given whether it runs elevated; none when
/// it may. Not knowing refuses too.
fn refusal(elevated: io::Result<bool>) -> Option<Refusal> {
    match elevated {
        Ok(false) => None,
        Ok(true) => Some(Refusal::Elevated),
        Err(err) => Some(Refusal::Unknown(err)),
    }
}

/// Why an elevated program does not start.
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
pub fn language() -> Language {
    display_languages().map_or(Language::English, |names| {
        Language::from_tags(names.iter().map(String::as_str))
    })
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
