//! The tray's menu labels and title line in the user's language. The menu
//! is built in English; [`label`] gives each label in the language shown.
//! The tooltip's other lines stay English for now (#60).

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
mod tests {
    use std::sync::Arc;

    use auto_clicker_core::config::Config;
    use auto_clicker_core::controller::NoRecording;
    use auto_clicker_core::hotkeys::Unavailable;

    use super::*;

    fn view(activity: Activity) -> View {
        let config = Config::default();
        View {
            activity,
            settings: config.clicker,
            hotkey: Hotkey::Bound(config.hotkeys.toggle_clicking),
            record_hotkey: Hotkey::Bound(config.hotkeys.toggle_recording),
            no_recording: None,
            play_hotkey: Hotkey::Desktop(Some("Meta+P".to_owned())),
            playback: config.playback,
            recordings: Arc::from([]),
            playing: None,
            notice: None,
            problems: Vec::new(),
            routes: auto_clicker_core::controller::Routes::default(),
        }
    }

    #[test]
    fn english_is_as_built_and_swedish_has_every_menu_label() {
        let labels = [
            "Start clicking",
            "Stop clicking",
            "Record",
            "Stop recording",
            "Play",
            "Stop playing",
            "Settings",
            "Quit",
            "No recordings yet",
            NoRecording::DesktopHotkeys.label(),
            NoRecording::Keyboards(Unavailable::NoPermission).label(),
            NoRecording::Keyboards(Unavailable::NoKeyboard).label(),
        ];
        for english in labels {
            assert_eq!(label(english, Language::English), english);
            assert_ne!(label(english, Language::Swedish), english, "{english}");
        }
        assert_eq!(
            label("Recording 2026-11-09 18:04:05", Language::Swedish),
            "Inspelning 2026-11-09 18:04:05"
        );
    }

    #[test]
    fn the_title_line_speaks_the_language() {
        let waiting = view(Activity::WaitingForRelease);
        assert_eq!(title(&waiting, Language::English), waiting.title());
        assert_eq!(
            title(&waiting, Language::Swedish),
            "Autoklickare: börjar när du släpper Ctrl och Shift"
        );
        let pause = DESKTOP_RELEASE_PAUSE.as_secs();
        assert_eq!(
            title(&view(Activity::PlayWaitingForRelease), Language::Swedish),
            format!("Autoklickare: spelar upp {pause} s efter att du släppt snabbtangenten")
        );
        let every = [
            Activity::Idle,
            Activity::StartingIn(3),
            Activity::Opening,
            Activity::Clicking,
            Activity::RecordingIn(2),
            Activity::Recording,
            Activity::PlayingIn(1),
            Activity::PlayOpening,
            Activity::Playing,
        ];
        for activity in every {
            let shown = title(&view(activity), Language::Swedish);
            assert!(shown.starts_with("Autoklickare: "), "{shown}");
            assert_ne!(shown, view(activity).title());
        }
    }
}
