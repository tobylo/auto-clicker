//! The release guarantee, proved against the fake backend.

use std::panic::{AssertUnwindSafe, catch_unwind};

use super::fake::{Call, FakeBackend, FakeLog};
use super::{Button, Held, Input, InputBackend, Key, PressState};

const KEY_A: u16 = 30;

fn key_a() -> Key {
    Key::new(KEY_A).expect("KEY_A is a keyboard key")
}

fn up(button: Button) -> Call {
    Call::Button(button, PressState::Up)
}

fn down(button: Button) -> Call {
    Call::Button(button, PressState::Down)
}

/// The calls made after the first `skip`.
fn calls_after(log: &FakeLog, skip: usize) -> Vec<Call> {
    log.calls().into_iter().skip(skip).collect()
}

#[test]
fn release_all_releases_every_held_button_and_key_and_nothing_else() {
    let (mut input, log) = FakeBackend::open();
    input.press_button(Button::Left).unwrap();
    input.press_button(Button::Right).unwrap();
    input.press_key(key_a()).unwrap();
    input.release_button(Button::Right).unwrap();
    let before = log.calls().len();

    input.release_all().unwrap();

    assert_eq!(
        calls_after(&log, before),
        [up(Button::Left), Call::Key(key_a(), PressState::Up)]
    );
    assert_eq!(input.held().count(), 0);
}

#[test]
fn release_all_releases_in_a_fixed_order_whatever_the_press_order() {
    let (mut input, log) = FakeBackend::open();
    input.press_key(key_a()).unwrap();
    input.press_button(Button::Right).unwrap();
    input.press_button(Button::Left).unwrap();

    input.release_all().unwrap();

    assert_eq!(
        calls_after(&log, 3),
        [
            up(Button::Left),
            up(Button::Right),
            Call::Key(key_a(), PressState::Up)
        ]
    );
}

#[test]
fn a_press_whose_write_failed_is_still_released() {
    let (mut input, log) = FakeBackend::open();
    log.fail_once(down(Button::Left));

    assert!(input.press_button(Button::Left).is_err());
    assert!(input.is_held(Held::Button(Button::Left)));

    input.release_all().unwrap();
    assert_eq!(log.calls().last(), Some(&up(Button::Left)));
    assert!(!input.is_held(Held::Button(Button::Left)));
}

#[test]
fn release_all_keeps_going_after_a_failure_and_returns_the_first_error() {
    let (mut input, log) = FakeBackend::open();
    input.press_button(Button::Left).unwrap();
    input.press_button(Button::Right).unwrap();
    input.press_key(key_a()).unwrap();
    log.fail_once(up(Button::Left));
    log.fail_once(up(Button::Right));
    let before = log.calls().len();

    let err = input.release_all().unwrap_err();

    assert!(err.to_string().contains("Left"), "first error: {err}");
    assert_eq!(
        calls_after(&log, before),
        [
            up(Button::Left),
            up(Button::Right),
            Call::Key(key_a(), PressState::Up)
        ]
    );
    let still_held: Vec<Held> = input.held().collect();
    assert_eq!(
        still_held,
        [Held::Button(Button::Left), Held::Button(Button::Right)]
    );

    input.release_all().unwrap();
    assert_eq!(input.held().count(), 0);
}

#[test]
fn dropping_the_input_releases_everything_held() {
    let (mut input, log) = FakeBackend::open();
    input.press_button(Button::Middle).unwrap();
    input.press_key(key_a()).unwrap();
    let before = log.calls().len();

    drop(input);

    assert_eq!(
        calls_after(&log, before),
        [up(Button::Middle), Call::Key(key_a(), PressState::Up)]
    );
}

#[test]
fn drop_retries_a_release_that_failed() {
    let (mut input, log) = FakeBackend::open();
    input.press_button(Button::Side).unwrap();
    log.fail_once(up(Button::Side));
    assert!(input.release_button(Button::Side).is_err());
    assert!(input.is_held(Held::Button(Button::Side)));

    drop(input);

    assert_eq!(calls_after(&log, 1), [up(Button::Side), up(Button::Side)]);
}

#[test]
fn a_panic_while_buttons_are_held_releases_them_during_unwinding() {
    let (input, log) = FakeBackend::open();

    let result = catch_unwind(AssertUnwindSafe(move || {
        let mut input = input;
        input.press_button(Button::Left).unwrap();
        input.press_key(key_a()).unwrap();
        panic!("engine bug while holding input");
    }));

    assert!(result.is_err());
    assert_eq!(
        calls_after(&log, 2),
        [up(Button::Left), Call::Key(key_a(), PressState::Up)]
    );
}

#[test]
fn pressing_a_held_button_again_needs_only_one_release() {
    let (mut input, log) = FakeBackend::open();
    input.press_button(Button::Left).unwrap();
    input.press_button(Button::Left).unwrap();
    input.press_key(key_a()).unwrap();
    input.press_key(key_a()).unwrap();

    input.release_button(Button::Left).unwrap();
    input.release_key(key_a()).unwrap();
    let before = log.calls().len();
    input.release_all().unwrap();
    drop(input);

    assert_eq!(calls_after(&log, before), []);
}

#[test]
fn releasing_a_button_that_is_not_held_still_writes_the_release() {
    let (mut input, log) = FakeBackend::open();

    input.release_button(Button::Extra).unwrap();

    assert_eq!(log.calls(), [up(Button::Extra)]);
    assert_eq!(input.held().count(), 0);
}

#[test]
fn motion_and_scrolling_pass_through_and_hold_nothing() {
    let (mut input, log) = FakeBackend::open();
    let position = super::AbsolutePosition { x: 0, y: u16::MAX };

    input.move_relative(5, -3).unwrap();
    input.move_absolute(position).unwrap();
    input.scroll(-2).unwrap();

    assert_eq!(
        log.calls(),
        [
            Call::MoveRelative(5, -3),
            Call::MoveAbsolute(position),
            Call::Scroll(-2)
        ]
    );
    assert_eq!(input.held().count(), 0);
}

#[test]
fn a_boxed_backend_chosen_at_run_time_keeps_the_guarantee() {
    let (backend, log) = FakeBackend::new();
    let boxed: Box<dyn InputBackend + Send> = Box::new(backend);
    let mut input = Input::new(boxed);
    let position = super::AbsolutePosition { x: 1, y: 2 };
    input.press_button(Button::Right).unwrap();
    input.press_key(key_a()).unwrap();
    input.move_relative(1, 1).unwrap();
    input.move_absolute(position).unwrap();
    input.scroll(1).unwrap();

    drop(input);

    assert_eq!(
        log.calls(),
        [
            down(Button::Right),
            Call::Key(key_a(), PressState::Down),
            Call::MoveRelative(1, 1),
            Call::MoveAbsolute(position),
            Call::Scroll(1),
            up(Button::Right),
            Call::Key(key_a(), PressState::Up),
        ]
    );
}
