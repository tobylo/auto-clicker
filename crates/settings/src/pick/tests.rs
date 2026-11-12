use std::cell::RefCell;
use std::rc::Rc;

use auto_clicker_core::recording::Output;
use auto_clicker_core::window::PICK_TIME;
use slint::platform::WindowEvent;

use super::*;

const PANEL: Rect = Rect {
    x: 0,
    y: 0,
    width: 2048,
    height: 1280,
};
/// DP-1 at 1.5 beside the panel, as KWin has it (decision 0001).
const DP1: Rect = Rect {
    x: 2048,
    y: 0,
    width: 2293,
    height: 960,
};

fn screens() -> Layout {
    Layout::new(vec![
        Output::new(PANEL, 1250).unwrap(),
        Output::new(DP1, 1500).unwrap(),
    ])
    .unwrap()
}

fn monitor(left: i64, top: i64, name: &'static str) -> Monitor<&'static str> {
    Monitor {
        left,
        top,
        monitor: name,
    }
}

/// The screens each monitor was matched to, by name.
fn matched(monitors: Vec<Monitor<&'static str>>) -> Option<Vec<(&'static str, Rect)>> {
    let targets = targets(monitors, &screens())?;
    Some(targets.into_iter().map(|t| (t.monitor, t.on)).collect())
}

#[test]
fn monitors_match_the_screens_by_their_corners() {
    assert_eq!(
        matched(vec![monitor(2048, 0, "DP-1"), monitor(0, 0, "eDP-1")]),
        Some(vec![("DP-1", DP1), ("eDP-1", PANEL)])
    );
    let unmatched = [
        // A screen of the layout no monitor shows.
        vec![monitor(0, 0, "eDP-1")],
        // A monitor on no screen of the layout.
        vec![
            monitor(0, 0, "eDP-1"),
            monitor(2048, 0, "DP-1"),
            monitor(0, 1280, "HDMI-1"),
        ],
        // DP-1 where KScreen's rounding would put a monitor right of it.
        vec![monitor(0, 0, "eDP-1"), monitor(2049, 0, "DP-1")],
        vec![],
    ];
    for monitors in unmatched {
        let corners: Vec<_> = monitors.iter().map(|m| (m.left, m.top)).collect();
        assert!(matched(monitors).is_none(), "{corners:?}");
    }
}

#[test]
fn a_click_is_its_screens_corner_plus_its_place_rounded_and_kept_on_the_screen() {
    let cases = [
        ((PANEL, 0.0, 0.0), (0, 0)),
        ((PANEL, 1000.4, 640.5), (1000, 641)),
        ((DP1, 0.25, 0.0), (2048, 0)),
        ((DP1, 952.5, 700.75), (3001, 701)),
        // The overlay can reach past KWin's whole-pixel size.
        ((DP1, 2292.75, 959.6), (4340, 959)),
        ((DP1, 2293.0, 960.0), (4340, 959)),
        // What no click gives, kept on the screen anyway.
        ((DP1, -3.0, f32::NAN), (2048, 0)),
        ((DP1, f32::INFINITY, f32::NEG_INFINITY), (4340, 0)),
    ];
    for ((on, x, y), (to_x, to_y)) in cases {
        assert_eq!(point_on(on, x, y), Point { x: to_x, y: to_y }, "{x},{y}");
    }
}

/// What the pick told, in order.
type Told = Rc<RefCell<Vec<PickEnd>>>;

fn open(time: Duration) -> (Pick, Told) {
    let told = Told::default();
    let telling = Rc::clone(&told);
    let targets = vec![
        Target {
            on: PANEL,
            monitor: (),
        },
        Target {
            on: DP1,
            monitor: (),
        },
    ];
    let place: Placer<()> = |_, ()| Ok(None);
    let pick = Pick::open(targets, place, time, move |how| {
        telling.borrow_mut().push(how)
    });
    (pick.unwrap(), told)
}

fn shown(pick: &Pick) -> Vec<bool> {
    pick.windows
        .iter()
        .map(|w| w.window().is_visible())
        .collect()
}

/// Asserts that dropping `pick` closes every overlay it opened.
fn closes_every_overlay(pick: Pick) {
    let windows: Vec<PickWindow> = pick.windows.iter().map(|w| w.clone_strong()).collect();
    drop(pick);
    for window in windows {
        assert!(!window.window().is_visible());
    }
}

/// The overlay's lifecycle, on the headless platform of the app's test:
/// Slint's platform is set once for the process.
pub(crate) fn every_end_is_told_once_and_closes_every_overlay() {
    let (pick, told) = open(PICK_TIME);
    assert_eq!(shown(&pick), [true, true]);
    pick.windows[1].invoke_picked(952.5, 700.75);
    // A second click or an Esc before the app closed it tells nothing more.
    pick.windows[0].invoke_picked(1.0, 1.0);
    pick.windows[0].invoke_cancelled();
    let point = Point { x: 3001, y: 701 };
    assert_eq!(*told.borrow(), [PickEnd::At { point, on: DP1 }]);
    closes_every_overlay(pick);

    let (pick, told) = open(PICK_TIME);
    pick.windows[0].invoke_cancelled();
    assert_eq!(*told.borrow(), [PickEnd::Cancelled]);
    closes_every_overlay(pick);

    let (pick, told) = open(PICK_TIME);
    pick.windows[1]
        .window()
        .dispatch_event(WindowEvent::CloseRequested);
    assert_eq!(*told.borrow(), [PickEnd::Cancelled]);
    closes_every_overlay(pick);

    let (pick, told) = open(Duration::ZERO);
    assert_eq!(*told.borrow(), []);
    slint::platform::update_timers_and_animations();
    assert_eq!(*told.borrow(), [PickEnd::TimedOut]);
    closes_every_overlay(pick);

    // Dropped before it ended (the settings window closed): nothing is told.
    let (pick, told) = open(Duration::ZERO);
    closes_every_overlay(pick);
    slint::platform::update_timers_and_animations();
    assert_eq!(*told.borrow(), []);
}
