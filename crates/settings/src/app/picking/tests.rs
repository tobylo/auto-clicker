use auto_clicker_core::config::{Config, Draft};
use auto_clicker_core::controller::{Activity, Hotkey, Routes};
use auto_clicker_core::message::NoScreens;
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

fn view(activity: Activity, screens: Result<Layout, NoScreens>) -> WindowView {
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
        since: Since {
            runs: 1,
            layouts: 1,
        },
        answer: None,
    }
}

#[test]
fn a_pick_begins_only_idle_on_known_screens_with_nothing_else_asked() {
    let idle = view(Activity::Idle, Ok(screens(2048)));
    let since = Since {
        runs: 1,
        layouts: 1,
    };
    assert_eq!(
        pickable(Some(&idle), true, false),
        Some((screens(2048), since))
    );
    let unknown = view(Activity::Idle, Err(NoScreens::Lost));
    let clicking = view(Activity::Clicking, Ok(screens(2048)));
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
fn a_pick_ends_when_the_program_leaves_idle_a_run_begins_or_the_screens_change() {
    let begun = Since {
        runs: 1,
        layouts: 1,
    };
    let with = |activity, runs, layouts| {
        let mut shown = view(activity, Ok(screens(2048)));
        shown.since = Since { runs, layouts };
        shown
    };
    let changed = Some(Note::Unavailable(Unavailability::ScreensChanged));
    let cases = [
        (with(Activity::Idle, 1, 1), None),
        (with(Activity::Clicking, 1, 1), Some(Note::Busy)),
        (with(Activity::Recording, 1, 1), Some(Note::Busy)),
        // A run that began and ended between two views.
        (with(Activity::Idle, 2, 1), Some(Note::Busy)),
        // Screens that changed, even back to the same layout.
        (with(Activity::Idle, 1, 2), changed.clone()),
        (view(Activity::Idle, Err(NoScreens::Lost)), None),
    ];
    for (now, ends) in cases {
        assert_eq!(ends_pick(begun, &now), ends, "{now:?}");
    }
}
