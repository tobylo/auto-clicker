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
        current: None,
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

mod every_message;
mod samples;
mod wording;
