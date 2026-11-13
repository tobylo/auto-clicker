//! Picking a point through the controller: the view shows the screens as
//! KWin has them, and a pick is located on them only while idle.

use std::fs;

use super::settings::{draft, setup_with};
use super::*;
use crate::clicker::Position;
use crate::config::{ModeName, PositionDraft};
use crate::input::AbsolutePosition;
use crate::message::Unavailability;
use crate::recording::tests::positions::{screen, two_screens};
use crate::recording::{Layout, Rect};
use crate::window::{Ask, Located, Outcome, Since, Spot};

const DP1: Rect = Rect {
    x: 2048,
    y: 0,
    width: 3440,
    height: 1440,
};

fn locate(x: u32, y: u32) -> Ask {
    locate_since(x, y, Since::default())
}

fn locate_since(x: u32, y: u32, since: Since) -> Ask {
    Ask::Locate {
        at: Spot::new(f64::from(x), f64::from(y)).unwrap(),
        on: DP1,
        since,
    }
}

fn with_layout(layout: Option<Layout>) -> Shared {
    let shared = Shared::default();
    shared.with(|r| r.layout = layout);
    shared
}

#[tokio::test(start_paused = true)]
async fn a_point_on_the_screens_shown_is_located_on_them() {
    local(async {
        let mut s = setup_with(None, with_layout(Some(two_screens()))).await;
        assert_eq!(s.window.view().screens, Ok(two_screens()));
        // round(3768 * 65536 / 5488) = 44996; round(720 * 65536 / 1440) = 32768.
        let located = Outcome::Located(Located {
            x: 44996,
            y: 32768,
            screens: two_screens(),
        });
        assert_eq!(s.window.ask(&s.d, locate(3768, 720)).await, located);
        // Nothing is saved or pressed by a pick.
        assert!(!s.path.exists());
        assert_eq!(s.d.shared.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_view_follows_the_screens_and_a_pick_on_old_ones_is_refused() {
    local(async {
        let mut s = setup_with(None, with_layout(Some(two_screens()))).await;
        let begun = s.window.view().since;
        s.d.shared.with(|r| r.layout = None);
        s.d.send(Message::LayoutChanged).await;
        assert_eq!(s.window.view().screens, Err(NoScreens::NoDisplay));
        let unknown = Outcome::Unavailable(Unavailability::ScreensUnknown(NoScreens::NoDisplay));
        let now = s.window.view().since;
        assert_eq!(
            s.window.ask(&s.d, locate_since(3768, 720, now)).await,
            unknown
        );
        // DP-1 moved below the panel.
        let moved = Layout::new(vec![
            screen([0, 0, 2048, 1280], 1250),
            screen([0, 1280, 3440, 1440], 1000),
        ])
        .unwrap();
        s.d.shared.with(|r| r.layout = Some(moved.clone()));
        s.d.send(Message::LayoutChanged).await;
        assert_eq!(s.window.view().screens, Ok(moved));
        let changed = Outcome::Unavailable(Unavailability::ScreensChanged);
        let now = s.window.view().since;
        assert_eq!(
            s.window.ask(&s.d, locate_since(3768, 720, now)).await,
            changed
        );
        // Back as they were, as when two equal screens swap places: a pick
        // begun before the change is refused, one begun now is not.
        s.d.shared.with(|r| r.layout = Some(two_screens()));
        s.d.send(Message::LayoutChanged).await;
        assert_eq!(
            s.window.ask(&s.d, locate_since(3768, 720, begun)).await,
            changed
        );
        let now = s.window.view().since;
        let located = s.window.ask(&s.d, locate_since(3768, 720, now)).await;
        assert!(matches!(located, Outcome::Located(_)), "{located:?}");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn no_point_is_located_while_the_program_clicks_or_from_a_run_since() {
    local(async {
        let mut s = setup_with(None, with_layout(Some(two_screens()))).await;
        let begun = s.window.view().since;
        s.d.start_by_hotkey().await;
        assert_eq!(s.window.ask(&s.d, locate(3768, 720)).await, Outcome::Busy);
        s.d.send(Message::Stop).await;
        // Idle again, but a run's click may be the one picked.
        let stale = s.window.ask(&s.d, locate_since(3768, 720, begun)).await;
        assert_eq!(stale, Outcome::Busy);
        let now = s.window.view().since;
        assert_eq!(now.runs, begun.runs + 1);
        let located = s.window.ask(&s.d, locate_since(3768, 720, now)).await;
        assert!(matches!(located, Outcome::Located(_)), "{located:?}");
    })
    .await;
}

// Clicking at a picked point

const AT: AbsolutePosition = AbsolutePosition { x: 44996, y: 32768 };

/// A daemon holding the left button at `AT`, picked on `picked` (typed in
/// when `None`), its platform reporting `now`; the tray starts at once.
async fn holding_at(picked: Option<Layout>, now: Option<Layout>, mode: Mode) -> Daemon {
    let mut config = Config {
        tray_start_delay: Duration::ZERO,
        ..Config::default()
    };
    config.clicker.mode = mode;
    config.clicker.position = Position::Fixed(AT);
    config.fixed_screens = picked;
    daemon_with(config, with_layout(now), true).await
}

fn moved() -> Layout {
    Layout::new(vec![
        screen([0, 0, 2048, 1280], 1250),
        screen([0, 1280, 3440, 1440], 1000),
    ])
    .unwrap()
}

/// The panel at another scale: the same places in logical pixels.
fn rescaled() -> Layout {
    Layout::new(vec![
        screen([0, 0, 2048, 1280], 1500),
        screen([2048, 0, 3440, 1440], 1000),
    ])
    .unwrap()
}

const CHANGED: &str =
    "the screens changed since the fixed point was picked: pick it again in Settings";
const UNKNOWN: &str = "the screen layout is not known, so the picked point cannot be checked";

#[tokio::test(start_paused = true)]
async fn clicking_at_a_picked_point_starts_only_on_its_screens() {
    local(async {
        let cases = [
            (Some(moved()), Some(format!("Not started: {CHANGED}"))),
            (None, Some(format!("Not started: {UNKNOWN}"))),
            (Some(rescaled()), None),
            (Some(two_screens()), None),
        ];
        for (now, refused) in cases {
            let d = holding_at(Some(two_screens()), now.clone(), Mode::HoldButton).await;
            d.send(Message::Start).await;
            match refused {
                Some(why) => {
                    d.assert_idle_after(&[]);
                    assert_eq!(d.notice(), why, "{now:?}");
                }
                None => {
                    assert_eq!(d.activity(), Activity::Clicking, "{now:?}");
                    assert_eq!(d.shared.calls(), [Call::MoveAbsolute(AT), LEFT_DOWN]);
                }
            }
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_hotkey_start_is_refused_on_other_screens_too() {
    local(async {
        let d = holding_at(Some(two_screens()), Some(moved()), Mode::HoldButton).await;
        d.press_hotkey().await;
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        d.assert_idle_after(&[]);
        assert_eq!(d.notice(), format!("Not started: {CHANGED}"));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_typed_point_or_a_key_mode_starts_on_any_screens() {
    local(async {
        let typed = holding_at(None, Some(moved()), Mode::HoldButton).await;
        typed.send(Message::Start).await;
        assert_eq!(typed.activity(), Activity::Clicking);
        let key = holding_at(Some(two_screens()), None, Mode::HoldKey).await;
        key.send(Message::Start).await;
        assert_eq!(key.activity(), Activity::Clicking);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn clicking_stops_when_its_screens_change_and_not_when_they_rescale() {
    local(async {
        for (now, why) in [(Some(moved()), CHANGED), (None, UNKNOWN)] {
            let d = holding_at(Some(two_screens()), Some(two_screens()), Mode::HoldButton).await;
            d.send(Message::Start).await;
            d.set_layout(Some(rescaled()));
            d.send(Message::LayoutChanged).await;
            assert_eq!(d.activity(), Activity::Clicking);
            d.set_layout(now);
            d.send(Message::LayoutChanged).await;
            d.assert_idle_after(&[Call::MoveAbsolute(AT), LEFT_DOWN, LEFT_UP]);
            assert_eq!(d.notice(), format!("Clicking stopped: {why}"));
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_countdown_to_clicking_ends_when_its_screens_change() {
    local(async {
        let mut config = Config::default();
        config.clicker.position = Position::Fixed(AT);
        config.fixed_screens = Some(two_screens());
        let d = daemon_with(config, with_layout(Some(two_screens())), true).await;
        d.send(Message::Start).await;
        assert!(matches!(d.activity(), Activity::StartingIn(_)));
        d.set_layout(Some(moved()));
        d.send(Message::LayoutChanged).await;
        sleep(Duration::from_secs(10)).await;
        d.settle().await;
        d.assert_idle_after(&[]);
        assert_eq!(d.notice(), format!("Clicking stopped: {CHANGED}"));
    })
    .await;
}

// Saving a picked point

/// The point `ask` located, as a draft holding the button down there.
fn holding_picked(located: &Outcome) -> crate::config::Draft {
    let Outcome::Located(Located { x, y, screens }) = located else {
        panic!("{located:?}");
    };
    draft(|d| {
        d.clicker.mode = ModeName::HoldButton;
        d.clicker.position = PositionDraft::Fixed {
            x: *x,
            y: *y,
            screens: Some(screens.clone()),
        };
    })
}

#[tokio::test(start_paused = true)]
async fn a_saved_pick_is_written_with_its_screens_and_used_at_once() {
    local(async {
        let mut s = setup_with(None, with_layout(Some(two_screens()))).await;
        let located = s.window.ask(&s.d, locate(3768, 720)).await;
        let saved = s.window.save(&s.d, holding_picked(&located)).await;
        assert_eq!(saved, Outcome::Saved);
        let text = fs::read_to_string(&s.path).unwrap();
        assert!(
            text.contains(
                "position = { x = 44996, y = 32768, screens = [\
                 { x = 0, y = 0, width = 2048, height = 1280, scale_milli = 1250 }, \
                 { x = 2048, y = 0, width = 3440, height = 1440, scale_milli = 1000 }] }"
            ),
            "{text}"
        );
        // The running daemon checks the saved screens without a restart.
        s.d.set_layout(Some(moved()));
        s.d.press_hotkey().await;
        s.d.hotkey(HotkeyEvent::ModifiersReleased).await;
        s.d.assert_idle_after(&[]);
        assert_eq!(s.d.notice(), format!("Not started: {CHANGED}"));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn picking_again_on_new_screens_replaces_the_old_ones_at_once() {
    local(async {
        let text = "version = 1\n[clicker.position]\nx = 1\ny = 2\nscreens = [\
                    { x = 0, y = 0, width = 2048, height = 1280, scale_milli = 1250 }]\n";
        let mut s = setup_with(Some(text), with_layout(Some(two_screens()))).await;
        let located = s.window.ask(&s.d, locate(3768, 720)).await;
        let saved = s.window.save(&s.d, holding_picked(&located)).await;
        assert_eq!(saved, Outcome::Saved);
        s.d.start_by_hotkey().await;
        assert_eq!(s.d.shared.calls(), [Call::MoveAbsolute(AT), LEFT_DOWN]);
    })
    .await;
}
