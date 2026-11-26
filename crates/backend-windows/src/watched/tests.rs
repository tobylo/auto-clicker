use std::collections::VecDeque;
use std::io;

use auto_clicker_core::input::TokioTimer;

use super::*;

const OPEN: Context = Context {
    desktop_has_input: true,
    foreground: 7,
    foreground_allowed: true,
};
const ELEVATED: Context = Context {
    foreground_allowed: false,
    ..OPEN
};
const LOCKED: Context = Context {
    desktop_has_input: false,
    ..OPEN
};

/// Output that notes each call and whether the table owed the item at that
/// moment, and refuses the calls it is told to.
struct Noting {
    table: Arc<Table>,
    noted: Vec<(Held, PressState, bool)>,
    refuse: Vec<(Held, PressState)>,
}

impl Noting {
    fn note(&mut self, item: Held, state: PressState) -> Result<(), InputError> {
        self.noted.push((item, state, self.table.owes(item)));
        match self.refuse.iter().position(|&call| call == (item, state)) {
            Some(at) => {
                self.refuse.remove(at);
                Err(InputError::Write(io::Error::other("refused")))
            }
            None => Ok(()),
        }
    }
}

impl InputBackend for Noting {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        self.note(Held::Button(button), state)
    }
    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError> {
        self.note(Held::Key(key), state)
    }
    fn move_relative(&mut self, _: i32, _: i32) -> Result<(), InputError> {
        Ok(())
    }
    fn move_absolute(&mut self, _: AbsolutePosition) -> Result<(), InputError> {
        Ok(())
    }
    fn scroll(&mut self, _: i32) -> Result<(), InputError> {
        Ok(())
    }
    fn run_timer(&mut self) -> Result<Box<dyn RunTimer>, InputError> {
        Ok(Box::new(TokioTimer::default()))
    }
}

/// What is in front at each question, in turn; open once the list is out.
struct Script(VecDeque<Context>);

impl Probe for Script {
    fn context(&mut self) -> Context {
        self.0.pop_front().unwrap_or(OPEN)
    }
}

fn w() -> Held {
    Held::Key(Key::new(17).unwrap())
}

fn left() -> Held {
    Held::Button(Button::Left)
}

fn watched(
    contexts: &[Context],
    refuse: &[(Held, PressState)],
) -> (Input<Watched<Noting, Script>>, Arc<Table>) {
    let table = Arc::new(Table::private());
    let output = Noting {
        table: Arc::clone(&table),
        noted: Vec::new(),
        refuse: refuse.to_vec(),
    };
    let probe = Script(contexts.iter().copied().collect());
    (
        Input::new(Watched::over(output, probe, Arc::clone(&table))),
        table,
    )
}

#[test]
fn a_press_is_marked_before_it_is_sent_and_cleared_once_its_release_arrived() {
    let (mut input, table) = watched(&[], &[]);
    input.press_key(Key::new(17).unwrap()).unwrap();
    input.press_button(Button::Left).unwrap();
    assert!(table.owes(w()) && table.owes(left()));
    input.release_all().unwrap();
    assert!(!table.owes(w()) && !table.owes(left()));
    let noted = &input.backend().output.noted;
    assert_eq!(
        noted[..2],
        [
            (w(), PressState::Down, true),
            (left(), PressState::Down, true)
        ]
    );
}

#[test]
fn a_press_that_failed_stays_marked_until_its_release_arrives() {
    let (mut input, table) = watched(&[], &[(w(), PressState::Down)]);
    assert!(input.press_key(Key::new(17).unwrap()).is_err());
    assert!(table.owes(w()));
    input.release_all().unwrap();
    assert!(!table.owes(w()));
}

/// A case's name, what is in front at each question, and what Windows
/// refuses.
type Case = (&'static str, Vec<Context>, Vec<(Held, PressState)>);

#[test]
fn a_release_that_may_not_have_arrived_stays_owed_and_the_run_goes_on() {
    // Blocked before the send, after it, by a change of window in between,
    // and refused by Windows in an open context.
    let other = Context {
        foreground: 8,
        ..OPEN
    };
    let cases: [Case; 5] = [
        ("elevated before", vec![ELEVATED, OPEN], vec![]),
        ("elevated after", vec![OPEN, ELEVATED], vec![]),
        ("locked", vec![LOCKED, LOCKED], vec![(w(), PressState::Up)]),
        ("another window", vec![OPEN, other], vec![]),
        ("refused", vec![OPEN, OPEN], vec![(w(), PressState::Up)]),
    ];
    for (name, contexts, refuse) in cases {
        let (mut input, table) = watched(&contexts, &refuse);
        input.press_key(Key::new(17).unwrap()).unwrap();
        // Released as far as the run goes: the backend owns it now.
        assert!(input.release_key(Key::new(17).unwrap()).is_ok(), "{name}");
        assert!(!input.is_held(w()), "{name}");
        assert!(table.owes(w()), "{name}");
    }
}

#[test]
fn an_owed_release_that_arrives_later_clears_its_mark() {
    let (mut input, table) = watched(&[ELEVATED, ELEVATED], &[]);
    input.press_key(Key::new(17).unwrap()).unwrap();
    input.release_key(Key::new(17).unwrap()).unwrap();
    assert!(table.owes(w()));
    // Pressed and released again with the user's window in front.
    input.press_key(Key::new(17).unwrap()).unwrap();
    input.release_key(Key::new(17).unwrap()).unwrap();
    assert!(!table.owes(w()));
}

#[test]
fn moves_and_scrolls_leave_the_table_alone() {
    let (mut input, table) = watched(&[], &[]);
    input.move_relative(3, 4).unwrap();
    input.scroll(1).unwrap();
    assert_eq!(table.copy(), [0; crate::watchdog::TABLE_LEN]);
}
