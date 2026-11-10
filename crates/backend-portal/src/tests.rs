//! The backend and the clicking engine against a fake compositor (no portal
//! and no compositor in unit tests). They run on the real clock, since the
//! fake talks over a real socket; they assert what arrives and in which
//! order, with time bounds only as generous upper limits.

use std::future::{Future, pending};
use std::num::NonZeroU32;
use std::time::Duration;

use auto_clicker_core::clicker::{ClickerSettings, Interval, Mode, Position, Repeat, run};
use auto_clicker_core::input::{
    AbsolutePosition, Button, Held, Input, InputError, Key, PressState,
};
use auto_clicker_core::run::{Ended, RunEnd, Signals};
use tokio::task::LocalSet;
use tokio::time::{sleep, timeout};

use super::*;
use crate::fake_eis::{Command, FakeEis, Got, Region};

/// X02's laptop: eDP-1 beside the taller DP-1.
const TWO_MONITORS: [Region; 2] = [(0, 0, 2048, 1280), (2048, 0, 3440, 1440)];

/// Far longer than anything here should take; a bound, not a measure.
const BOUND: Duration = Duration::from_secs(2);

const LEFT: u32 = 0x110;
const KEY_A: u32 = 30;

/// Runs a test body on a local set: the fake compositor is a local task.
async fn local(body: impl Future<Output = ()>) {
    LocalSet::new().run_until(body).await;
}

/// The backend over a fake compositor's socket, connected and ready.
async fn connect(regions: &[Region]) -> (Input<PortalBackend>, FakeEis) {
    let (fake, socket) = FakeEis::start(regions);
    let all = capabilities(DeviceType::Keyboard | DeviceType::Pointer);
    let eis = timeout(BOUND, Eis::connect(socket, &all))
        .await
        .expect("connected in time")
        .expect("connected");
    let input = Input::new(PortalBackend {
        eis,
        _session: SessionGuard::none(),
    });
    (input, fake)
}

/// Lets the fake compositor read what was sent.
async fn arrived(fake: &FakeEis, count: usize) -> Vec<Got> {
    let waited = timeout(BOUND, async {
        while fake.events().len() < count {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await;
    let events = fake.events();
    assert!(waited.is_ok(), "only {events:?} arrived");
    events
}

fn key(code: u16) -> Key {
    Key::new(code).unwrap()
}

// ---------------------------------------------------------------------------
// Each method of the trait

#[tokio::test]
async fn every_button_arrives_with_its_linux_code() {
    local(async {
        let (mut input, fake) = connect(&TWO_MONITORS).await;
        for button in Button::ALL {
            input.press_button(button).unwrap();
            input.release_button(button).unwrap();
        }
        let expected: Vec<Got> = [0x110, 0x111, 0x112, 0x113, 0x114]
            .into_iter()
            .flat_map(|code| [Got::Button(code, true), Got::Button(code, false)])
            .collect();
        assert_eq!(arrived(&fake, 10).await, expected);
    })
    .await;
}

#[tokio::test]
async fn each_write_is_one_request_and_a_frame_and_emulation_starts_once() {
    local(async {
        let (mut input, fake) = connect(&TWO_MONITORS).await;
        input.press_key(key(30)).unwrap();
        input.release_key(key(30)).unwrap();
        input.move_relative(100, -50).unwrap();
        arrived(&fake, 3).await;
        assert_eq!(
            fake.got(),
            [
                Got::StartEmulating("keyboard"),
                Got::Key(KEY_A, true),
                Got::Frame("keyboard"),
                Got::Key(KEY_A, false),
                Got::Frame("keyboard"),
                Got::StartEmulating("pointer"),
                Got::Motion(100.0, -50.0),
                Got::Frame("pointer"),
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn absolute_points_span_every_output_and_scroll_flips_its_sign() {
    local(async {
        let (mut input, fake) = connect(&TWO_MONITORS).await;
        input
            .move_absolute(AbsolutePosition { x: 32768, y: 32768 })
            .unwrap();
        // Below the shorter laptop screen: onto its last row.
        input
            .move_absolute(AbsolutePosition { x: 0, y: 65535 })
            .unwrap();
        input.scroll(1).unwrap();
        input.scroll(-2).unwrap();
        assert_eq!(
            arrived(&fake, 4).await,
            [
                Got::Absolute(2744.0, 720.0),
                Got::Absolute(0.0, 1279.0),
                Got::Scroll(0, -120),
                Got::Scroll(0, 240),
            ]
        );
    })
    .await;
}

#[test]
fn scroll_units_saturate() {
    assert_eq!(scroll_units(i32::MAX), i32::MIN);
    assert_eq!(scroll_units(i32::MIN), i32::MAX);
    assert_eq!(scroll_units(0), 0);
}

#[test]
fn allowed_device_types_decide_the_devices_waited_for() {
    use DeviceCapability::*;
    assert_eq!(capabilities(DeviceType::Keyboard.into()), [Keyboard]);
    assert_eq!(
        capabilities(DeviceType::Pointer.into()),
        [Pointer, PointerAbsolute, Button, Scroll]
    );
    assert!(capabilities(BitFlags::empty()).is_empty());
}

// ---------------------------------------------------------------------------
// The engine over this backend

fn settings(mode: Mode, count: u32) -> ClickerSettings {
    ClickerSettings {
        mode,
        interval: Interval::new(Duration::from_millis(20)).unwrap(),
        key: key(30),
        repeat: Repeat::Times(NonZeroU32::new(count).unwrap()),
        ..ClickerSettings::default()
    }
}

fn never() -> impl Future<Output = ()> {
    pending()
}

/// Runs the engine with `settings` until it finishes or `stop` fires.
async fn run_engine(
    input: Input<PortalBackend>,
    settings: ClickerSettings,
    stop: impl Future<Output = ()>,
) -> RunEnd<PortalBackend> {
    let signals = Signals {
        stop,
        shutdown: never(),
    };
    timeout(
        BOUND,
        run(input, &settings, fastrand::Rng::with_seed(7), signals),
    )
    .await
    .expect("the run ended in time")
}

#[tokio::test]
async fn the_engine_clicks_double_clicks_and_taps_keys_through_the_portal() {
    let press = |code, down| Got::Button(code, down);
    let cases = [
        (
            Mode::Click,
            3,
            [press(LEFT, true), press(LEFT, false)].repeat(3),
        ),
        (
            Mode::DoubleClick,
            1,
            [press(LEFT, true), press(LEFT, false)].repeat(2),
        ),
        (
            Mode::KeyRepeat,
            2,
            [Got::Key(KEY_A, true), Got::Key(KEY_A, false)].repeat(2),
        ),
    ];
    for (mode, count, expected) in cases {
        local(async {
            let (input, fake) = connect(&TWO_MONITORS).await;
            let end = run_engine(input, settings(mode, count), never()).await;
            assert!(
                matches!(end.result, Ok(Ended::Finished)),
                "{mode:?}: {:?}",
                end.result
            );
            assert_eq!(arrived(&fake, expected.len()).await, expected, "{mode:?}");
        })
        .await;
    }
}

#[tokio::test]
async fn the_engine_moves_to_a_fixed_point_before_each_click() {
    local(async {
        let (input, fake) = connect(&TWO_MONITORS).await;
        let settings = ClickerSettings {
            position: Position::Fixed(AbsolutePosition { x: 32768, y: 32768 }),
            ..settings(Mode::Click, 2)
        };
        let end = run_engine(input, settings, never()).await;
        assert!(end.result.is_ok(), "{:?}", end.result);
        let click = [
            Got::Absolute(2744.0, 720.0),
            Got::Button(LEFT, true),
            Got::Button(LEFT, false),
        ];
        assert_eq!(arrived(&fake, 6).await, [click, click].concat());
    })
    .await;
}

#[tokio::test]
async fn holding_releases_on_stop() {
    for (mode, held) in [
        (Mode::HoldButton, Got::Button(LEFT, true)),
        (Mode::HoldKey, Got::Key(KEY_A, true)),
    ] {
        local(async {
            let (input, fake) = connect(&TWO_MONITORS).await;
            let stop = sleep(Duration::from_millis(50));
            let end = run_engine(input, settings(mode, 1), stop).await;
            assert!(matches!(end.result, Ok(Ended::Stopped)), "{:?}", end.result);
            let released = match held {
                Got::Button(code, _) => Got::Button(code, false),
                Got::Key(code, _) => Got::Key(code, false),
                _ => unreachable!(),
            };
            assert_eq!(arrived(&fake, 2).await, [held, released], "{mode:?}");
        })
        .await;
    }
}

// ---------------------------------------------------------------------------
// Losing the session

#[tokio::test]
async fn a_session_lost_mid_run_stops_a_held_button_at_once() {
    local(async {
        let (input, fake) = connect(&TWO_MONITORS).await;
        let lost = PortalOutput::lost(&input);
        // The controller stops a run when its devices are lost; here the
        // loss is the run's stop signal directly.
        let stop = async {
            lost.await;
        };
        let run = run_engine(input, settings(Mode::HoldButton, 1), stop);
        let disconnect = async {
            arrived(&fake, 1).await;
            fake.command(Command::Disconnect);
        };
        let (end, ()) = tokio::join!(run, disconnect);
        // The release fails on the lost session, so the run hands back no
        // input and reports it; the compositor released the button itself.
        assert!(end.input.is_none());
        assert!(
            matches!(&end.result, Err(InputError::SessionLost(_))),
            "{:?}",
            end.result
        );
    })
    .await;
}

#[tokio::test]
async fn after_a_disconnect_writes_fail_as_session_lost() {
    local(async {
        let (mut input, fake) = connect(&TWO_MONITORS).await;
        let lost = PortalOutput::lost(&input);
        fake.command(Command::Disconnect);
        let err = timeout(BOUND, lost).await.expect("the loss in time");
        assert!(
            matches!(&err, InputError::SessionLost(why) if why.contains("the session closed")),
            "{err:?}"
        );
        let write = input.press_button(Button::Left);
        assert!(
            matches!(write, Err(InputError::SessionLost(_))),
            "{write:?}"
        );
        // The press stays held, so a later release is tried again.
        assert!(input.is_held(Held::Button(Button::Left)));
    })
    .await;
}

#[tokio::test]
async fn a_pause_or_a_closed_socket_is_a_loss() {
    for command in [Command::Pause, Command::Close] {
        local(async {
            let (input, fake) = connect(&TWO_MONITORS).await;
            let lost = PortalOutput::lost(&input);
            let name = format!("{command:?}");
            fake.command(command);
            let err = timeout(BOUND, lost).await.expect("the loss in time");
            assert!(matches!(err, InputError::SessionLost(_)), "{name}: {err:?}");
        })
        .await;
    }
}

#[tokio::test]
async fn a_removed_device_alone_is_not_a_loss() {
    local(async {
        let (mut input, fake) = connect(&TWO_MONITORS).await;
        let lost = PortalOutput::lost(&input);
        fake.command(Command::RemoveAbsolute);
        // Nothing ends the watch; it keeps the events flowing meanwhile.
        let watched = timeout(Duration::from_millis(200), lost).await;
        assert!(watched.is_err(), "{watched:?}");
        // The other devices still work; the removed one says so.
        input.press_button(Button::Left).unwrap();
        input.release_button(Button::Left).unwrap();
        let moved = input.move_absolute(AbsolutePosition { x: 0, y: 0 });
        assert!(
            matches!(&moved, Err(InputError::Portal(why)) if why.contains("no device")),
            "{moved:?}"
        );
        assert_eq!(
            arrived(&fake, 2).await,
            [Got::Button(LEFT, true), Got::Button(LEFT, false)]
        );
    })
    .await;
}

#[tokio::test]
async fn dropping_the_input_releases_then_ends_the_connection() {
    local(async {
        let (mut input, fake) = connect(&TWO_MONITORS).await;
        input.press_button(Button::Right).unwrap();
        drop(input);
        let got = timeout(BOUND, async {
            loop {
                let got = fake.events();
                if got.contains(&Got::Disconnect) {
                    return got;
                }
                sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("disconnected in time");
        assert_eq!(
            got,
            [
                Got::Button(0x111, true),
                Got::Button(0x111, false),
                Got::Disconnect
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn a_compositor_that_ends_the_connection_while_connecting_fails_the_open() {
    local(async {
        let (fake, socket) = FakeEis::start(&TWO_MONITORS);
        fake.command(Command::Close);
        let all = capabilities(DeviceType::Keyboard | DeviceType::Pointer);
        let opened = timeout(BOUND, Eis::connect(socket, &all))
            .await
            .expect("in time");
        assert!(opened.is_err());
    })
    .await;
}

#[test]
fn press_states_map_to_libei() {
    assert_eq!(
        button_state(PressState::Down),
        ei::button::ButtonState::Press
    );
    assert_eq!(
        button_state(PressState::Up),
        ei::button::ButtonState::Released
    );
    assert_eq!(key_state(PressState::Down), ei::keyboard::KeyState::Press);
    assert_eq!(key_state(PressState::Up), ei::keyboard::KeyState::Released);
}

#[test]
fn reis_protocol_logging_is_refused() {
    use std::ffi::OsString;
    assert!(no_protocol_log(None).is_ok());
    assert!(no_protocol_log(Some(OsString::new())).is_ok());
    for value in ["1", "0"] {
        let refused = no_protocol_log(Some(OsString::from(value)));
        assert!(
            matches!(&refused, Err(InputError::PortalUnavailable(why)) if why.contains("REIS_DEBUG")),
            "{value}: {refused:?}"
        );
    }
}

#[tokio::test]
async fn removing_a_device_in_use_is_a_loss() {
    local(async {
        let (mut input, fake) = connect(&TWO_MONITORS).await;
        let lost = PortalOutput::lost(&input);
        input
            .move_absolute(AbsolutePosition { x: 0, y: 0 })
            .unwrap();
        arrived(&fake, 1).await;
        fake.command(Command::RemoveAbsolute);
        let err = timeout(BOUND, lost).await.expect("the loss in time");
        assert!(
            matches!(&err, InputError::SessionLost(why) if why.contains("removed")),
            "{err:?}"
        );
    })
    .await;
}
