use auto_clicker_core::config::{Config, Draft};
use auto_clicker_core::controller::{Activity, Hotkey, Routes};
use auto_clicker_core::recording::{Output, Rect};
use auto_clicker_core::window::ShownHotkeys;

use super::*;

fn screens(width: u32) -> Layout {
    let rect = Rect {
        x: 0,
        y: 0,
        width,
        height: 1280,
    };
    Layout::new(vec![Output::new(rect, 1250).unwrap()]).unwrap()
}

fn view(activity: Activity, screens: Option<Layout>) -> WindowView {
    let config = Config::default();
    let settings = Draft::from(&config);
    let shown = |binding| Hotkey::Bound(binding);
    WindowView {
        running: settings.input,
        routes: Routes::default(),
        settings,
        activity,
        hotkeys: ShownHotkeys {
            toggle_clicking: shown(config.hotkeys.toggle_clicking),
            toggle_recording: shown(config.hotkeys.toggle_recording),
            toggle_playback: shown(config.hotkeys.toggle_playback),
        },
        no_recording: None,
        problems: Vec::new(),
        recordings: Default::default(),
        screens,
        answer: None,
    }
}

#[test]
fn a_pick_begins_only_idle_on_known_screens_with_nothing_else_asked() {
    let idle = view(Activity::Idle, Some(screens(2048)));
    assert_eq!(pickable(Some(&idle), true, false), Some(screens(2048)));
    let unknown = view(Activity::Idle, None);
    let clicking = view(Activity::Clicking, Some(screens(2048)));
    let cases = [
        (None, true, false),
        (Some(&unknown), true, false),
        (Some(&clicking), true, false),
        // A request awaits its answer, or a question is open.
        (Some(&idle), false, false),
        // A pick is up already.
        (Some(&idle), true, true),
    ];
    for (last, may_ask, picking) in cases {
        assert_eq!(
            pickable(last, may_ask, picking),
            None,
            "{may_ask} {picking}"
        );
    }
}

#[test]
fn a_pick_ends_when_the_program_leaves_idle_or_the_screens_change() {
    let begun = view(Activity::Idle, Some(screens(2048)));
    let cases = [
        (view(Activity::Idle, Some(screens(2048))), None),
        (
            view(Activity::Clicking, Some(screens(2048))),
            Some(Note::Busy),
        ),
        (
            view(Activity::Recording, Some(screens(2048))),
            Some(Note::Busy),
        ),
        (
            view(Activity::Idle, Some(screens(2047))),
            Some(Note::Unavailable(Unavailability::ScreensChanged)),
        ),
        (
            view(Activity::Idle, None),
            Some(Note::Unavailable(Unavailability::ScreensChanged)),
        ),
    ];
    for (now, ends) in cases {
        assert_eq!(ends_pick(Some(&begun), &now), ends, "{now:?}");
    }
}
