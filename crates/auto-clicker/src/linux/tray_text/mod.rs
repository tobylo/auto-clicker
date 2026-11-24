//! What the tray shows, in the user's language: the menu labels, the title
//! line and the tooltip's lines, notices and problems. The menu is built
//! in English; [`label`] gives each label in the language shown. Core
//! gives the rest as values, whose English is their `Display`; the
//! Swedish forms are here, one module per family of messages.

mod lines;
mod notice;
mod problem;

use auto_clicker_core::controller::{Activity, Hotkey, View};
use auto_clicker_core::hotkeys::{Binding, DESKTOP_RELEASE_PAUSE};
use auto_clicker_core::locale::Language;

/// The menu labels with a Swedish form.
const SWEDISH: &[(&str, &str)] = &[
    ("Start clicking", "Börja klicka"),
    ("Stop clicking", "Sluta klicka"),
    ("Record", "Spela in"),
    ("Stop recording", "Sluta spela in"),
    ("Play", "Spela upp"),
    ("Stop playing", "Sluta spela upp"),
    ("Settings", "Inställningar"),
    ("Quit", "Avsluta"),
    ("No recordings yet", "Inga inspelningar än"),
    (
        "Record (needs hotkeys = \"kernel\")",
        "Spela in (kräver hotkeys = \"kernel\")",
    ),
    (
        "Record (needs the input group)",
        "Spela in (kräver gruppen input)",
    ),
    ("Record (needs a restart)", "Spela in (kräver en omstart)"),
    ("Record (movement only)", "Spela in (bara rörelser)"),
];

/// How a recording's label starts, and its Swedish form.
const RECORDING: (&str, &str) = ("Recording ", "Inspelning ");

/// `english`, a menu label, in `language`; as it is when it has no form
/// there.
pub fn label(english: &str, language: Language) -> String {
    if language == Language::English {
        return english.to_owned();
    }
    if let Some((_, swedish)) = SWEDISH.iter().find(|(known, _)| *known == english) {
        return (*swedish).to_owned();
    }
    match english.strip_prefix(RECORDING.0) {
        Some(when) => format!("{}{when}", RECORDING.1),
        None => english.to_owned(),
    }
}

/// The tooltip's lines for `view` in `language`.
pub fn details(view: &View, language: Language) -> Vec<String> {
    match language {
        Language::English => view.details(),
        Language::Swedish => lines::details(view),
    }
}

/// The tray's title line for `view` in `language`.
pub fn title(view: &View, language: Language) -> String {
    match language {
        Language::English => view.title(),
        Language::Swedish => swedish_title(view),
    }
}

fn swedish_title(view: &View) -> String {
    let state = match view.activity {
        Activity::Idle => "vilar".to_owned(),
        Activity::StartingIn(seconds) => format!("börjar om {seconds} s"),
        Activity::WaitingForRelease => format!("börjar {}", start_wait(&view.hotkey)),
        Activity::Opening => "startar; tillåt inmatning om skrivbordet frågar".to_owned(),
        Activity::Clicking => "klickar".to_owned(),
        Activity::RecordingIn(seconds) => format!("spelar in om {seconds} s"),
        Activity::Recording => "spelar in".to_owned(),
        Activity::PlayingIn(seconds) => format!("spelar upp om {seconds} s"),
        Activity::PlayWaitingForRelease => {
            format!("spelar upp {}", start_wait(&view.play_hotkey))
        }
        Activity::PlayOpening => {
            "startar uppspelningen; tillåt inmatning om skrivbordet frågar".to_owned()
        }
        Activity::Playing => "spelar upp".to_owned(),
    };
    format!("Autoklickare: {state}")
}

/// What a start from `hotkey` waits for, in Swedish.
fn start_wait(hotkey: &Hotkey) -> String {
    match hotkey {
        Hotkey::Bound(binding) => format!("när du släpper {}", modifiers_of(*binding)),
        Hotkey::Desktop(_) => format!(
            "{} s efter att du släppt snabbtangenten",
            DESKTOP_RELEASE_PAUSE.as_secs()
        ),
    }
}

/// "Ctrl" or "Ctrl och Shift"; "snabbtangenten" when it has no modifier.
fn modifiers_of(binding: Binding) -> String {
    let names: Vec<&str> = binding.modifiers().iter().map(|m| m.name()).collect();
    match names.split_last() {
        None => "snabbtangenten".to_owned(),
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} och {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests;
