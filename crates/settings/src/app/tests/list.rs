//! The recordings list and the actions on it, on Slint's software
//! renderer: what a click with Ctrl or Shift and the keys select, what
//! that enables, and what a delete sends.

use std::cell::RefCell;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::rc::Rc;

use auto_clicker_core::controller::{Activity, Listed};
use auto_clicker_core::window::{
    Answer, Ask, Deletion, Files, Outcome, RecordingsView, Removal, Request, Update, WindowView,
    parse,
};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, VecModel};

use super::super::App;
use crate::link::Link;
use crate::state::tests::{defaults, view};
use crate::ui::SettingsWindow;

/// The recordings list alone in a window: `ui/tests/recordings.slint`.
mod alone {
    include!(concat!(env!("OUT_DIR"), "/recordings.rs"));
}

/// The height of a row in the list.
const ROW: f32 = 28.0;

/// The list says which row a click hit, with Ctrl or Shift held, and which
/// keys move or select; it keeps the selection to Rust.
pub fn the_list_tells_clicks_with_their_keys_and_the_keys_that_select() {
    let list = alone::RecordingListWindow::new().unwrap();
    list.window().set_size(LogicalSize::new(400.0, 300.0));
    list.show().unwrap();
    let rows: Vec<alone::RecordingRow> = (0..5)
        .map(|n| alone::RecordingRow {
            text: format!("row {n}").into(),
            selected: false,
        })
        .collect();
    list.set_rows(Rc::new(VecModel::from(rows)).into());
    let told = Rc::new(RefCell::new(Vec::<String>::new()));
    let tell = |told: &Rc<RefCell<Vec<String>>>| {
        let told = Rc::clone(told);
        move |said: String| told.borrow_mut().push(said)
    };
    let said = tell(&told);
    list.on_pressed(move |index, toggle, range| said(format!("press {index} {toggle} {range}")));
    let said = tell(&told);
    list.on_step(move |by, extend| said(format!("step {by} {extend}")));
    let said = tell(&told);
    list.on_select_all(move || said("all".to_owned()));
    let window = list.window();
    let click = |row: u8| {
        let position = LogicalPosition::new(50.0, ROW * f32::from(row) + ROW / 2.0);
        let button = PointerEventButton::Left;
        window.dispatch_event(WindowEvent::PointerPressed { position, button });
        window.dispatch_event(WindowEvent::PointerReleased { position, button });
    };
    let key = |key: Key| {
        window.dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        window.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    };
    let held = |key: Key, then: &dyn Fn()| {
        window.dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        then();
        window.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    };

    click(0);
    held(Key::Control, &|| click(2));
    held(Key::Shift, &|| click(4));
    // The list has the keyboard now: the click gave it focus.
    key(Key::DownArrow);
    held(Key::Shift, &|| key(Key::UpArrow));
    key(Key::Home);
    key(Key::End);
    held(Key::Control, &|| {
        window.dispatch_event(WindowEvent::KeyPressed { text: "a".into() });
        window.dispatch_event(WindowEvent::KeyReleased { text: "a".into() });
    });
    // A plain A selects nothing.
    window.dispatch_event(WindowEvent::KeyPressed { text: "a".into() });

    assert_eq!(
        *told.borrow(),
        [
            "press 0 false false",
            "press 2 true false",
            "press 4 false true",
            "step 1 false",
            "step -1 true",
            "step -5 false",
            "step 5 false",
            "all",
        ]
    );
}

/// The list scrolls the row the arrows reached into view, down and back
/// up: a click at the bottom edge then hits it.
pub fn the_list_keeps_the_row_the_arrows_reached_in_view() {
    let list = alone::RecordingListWindow::new().unwrap();
    list.window().set_size(LogicalSize::new(400.0, 300.0));
    list.show().unwrap();
    let rows: Vec<alone::RecordingRow> = (0..30)
        .map(|n| alone::RecordingRow {
            text: format!("row {n}").into(),
            selected: false,
        })
        .collect();
    list.set_rows(Rc::new(VecModel::from(rows)).into());
    let hit = Rc::new(RefCell::new(Vec::<i32>::new()));
    let pressed = Rc::clone(&hit);
    list.on_pressed(move |index, _, _| pressed.borrow_mut().push(index));
    let window = list.window();
    let click = |y: f32| {
        let position = LogicalPosition::new(50.0, y);
        let button = PointerEventButton::Left;
        window.dispatch_event(WindowEvent::PointerPressed { position, button });
        window.dispatch_event(WindowEvent::PointerReleased { position, button });
    };
    let bottom = 300.0 - ROW / 2.0;

    click(bottom);
    list.set_cursor(29);
    slint::platform::update_timers_and_animations();
    click(bottom);
    list.set_cursor(0);
    slint::platform::update_timers_and_animations();
    click(ROW / 2.0);

    assert_eq!(*hit.borrow(), [10, 29, 0]);
}

/// A file made at `minute` past 18:00 on 2026-11-09.
fn file(minute: u32) -> auto_clicker_core::recording::FileName {
    auto_clicker_core::recording::FileName::parse(&format!("2026-11-09_18-{minute:02}-00.000.json"))
        .unwrap()
}

/// A view listing the recordings made at `minutes`, with `more` unlisted.
fn listing(minutes: &[u32], more: u64) -> WindowView {
    let mut shown = view(defaults(), None);
    shown.recordings = RecordingsView {
        listed: minutes
            .iter()
            .map(|&minute| Listed::new(file(minute), Some(format!("Loop {minute}"))))
            .collect(),
        more,
        current: None,
    };
    shown
}

/// The next request the window sent.
fn sent(read: &mut BufReader<UnixStream>) -> Request {
    let mut line = Vec::new();
    read.read_until(b'\n', &mut line).unwrap();
    parse(&line[..line.len() - 1]).unwrap()
}

/// Which rows are marked selected.
fn marked(window: &SettingsWindow) -> Vec<bool> {
    window
        .get_recordings()
        .iter()
        .map(|row| row.selected)
        .collect()
}

/// A window over a new link, its app, and the link's other end.
fn opened() -> (SettingsWindow, Rc<RefCell<App>>, BufReader<UnixStream>) {
    let window = SettingsWindow::new().unwrap();
    let (link, other) = Link::pair().unwrap();
    let app = App::start(&window, link, ',');
    (window, app, BufReader::new(other))
}

/// Whether the actions on one, on those selected and on all may go.
fn may(window: &SettingsWindow) -> (bool, bool, bool) {
    (
        window.get_may_use_one(),
        window.get_may_delete(),
        window.get_may_delete_all(),
    )
}

/// Rename and both plays take the one recording selected alone, and a
/// delete every one selected; nothing goes while something runs.
pub fn the_actions_follow_the_selection() {
    let (window, app, _read) = opened();
    let update = |shown: WindowView| app.borrow_mut().update(Update::View(Box::new(shown)));
    update(listing(&[3, 2, 1, 0], 2));
    let may = || may(&window);
    assert_eq!(may(), (false, false, true));

    window.invoke_press_recording(1, false, false);
    assert_eq!(may(), (true, true, true));
    assert_eq!(window.get_recording_name(), "Loop 2");
    window.invoke_press_recording(3, true, false);
    assert_eq!(may(), (false, true, true));
    assert_eq!(window.get_recording_name(), "");
    assert_eq!(marked(&window), [false, true, false, true]);
    window.invoke_press_recording(0, false, true);
    assert_eq!(marked(&window), [true, true, true, true]);
    window.invoke_step_recordings(1, false);
    assert_eq!(marked(&window), [false, true, false, false]);
    assert_eq!(window.get_recording_cursor(), 1);
    window.invoke_select_all_recordings();
    assert_eq!(window.get_selected_count(), 4);
    // Nothing while something runs.
    let mut running = listing(&[3, 2, 1, 0], 2);
    running.activity = Activity::Clicking;
    update(running);
    assert_eq!(may(), (false, false, false));
}

/// A delete sends every one selected and "Delete all" every one there is,
/// after one question each, in Swedish.
pub fn a_delete_asks_once_and_sends_the_selection_whole() {
    let (window, app, mut read) = opened();
    let update = |shown: WindowView| app.borrow_mut().update(Update::View(Box::new(shown)));
    let may = || may(&window);
    update(listing(&[3, 2, 1, 0], 2));
    window.invoke_press_recording(0, false, false);
    window.invoke_press_recording(2, true, false);
    window.invoke_delete_recordings();
    assert_eq!(
        window.get_confirm_text(),
        "Radera dessa 2 inspelningar för gott? Det går inte att ångra."
    );
    assert_eq!(may(), (false, false, false));
    window.invoke_confirm();
    let request = sent(&mut read);
    let both = Files::new(vec![file(3), file(1)]).unwrap();
    assert_eq!(request.ask, Ask::Delete(Deletion::Files(both)));

    let mut answered = listing(&[2, 0], 2);
    answered.answer = Some(Answer {
        id: request.id,
        outcome: Outcome::Deleted(Removal {
            deleted: 2,
            ..Removal::default()
        }),
    });
    update(answered.clone());
    assert_eq!(window.get_message(), "Raderade 2 inspelningar.");
    assert_eq!(marked(&window), [false, false]);
    assert_eq!(may(), (false, false, true));

    // One selected alone is asked about by its name.
    window.invoke_press_recording(1, false, false);
    window.invoke_delete_recordings();
    assert_eq!(
        window.get_confirm_text(),
        "Radera Loop 0 (2026-11-09 18:00:00) för gott? Det går inte att ångra."
    );
    // Another selection calls it off.
    window.invoke_press_recording(0, false, false);
    assert_eq!(window.get_confirm_text(), "");

    window.invoke_delete_all_recordings();
    assert_eq!(
        window.get_confirm_text(),
        "Radera alla 4 inspelningar, även de som inte visas, för gott? Det går inte att ångra."
    );
    // One more recording appeared: the count asked about is wrong now.
    let mut grown = listing(&[4, 2, 0], 2);
    grown.answer = answered.answer;
    update(grown);
    assert_eq!(window.get_confirm_text(), "");
    window.invoke_delete_all_recordings();
    window.invoke_confirm();
    assert_eq!(sent(&mut read).ask, Ask::Delete(Deletion::All));
}
