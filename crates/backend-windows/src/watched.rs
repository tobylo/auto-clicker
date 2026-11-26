//! The daemon's output while a watchdog stands by (M5 brief decisions 6
//! and 7): every press is marked in the shared table before it is sent, and
//! its mark is cleared only once the release is delivered. Should the
//! daemon die in between, the watchdog releases what is marked.
//!
//! Marking is a store to shared memory, nothing more, so a click never
//! waits on the watchdog. A release that may not have arrived stays marked:
//! the backend owns it from then on, and the run goes on (WN06b sends it
//! again; until then the watchdog sends it once the daemon ends).

use std::sync::Arc;

use auto_clicker_core::input::{
    AbsolutePosition, Button, Held, Input, InputBackend, InputError, Key, PressState, RunTimer,
};
use auto_clicker_core::log;

use crate::delivery::{Context, Delivery, context, judge};
use crate::output::Output;
use crate::watchdog::Table;

/// What is in front, asked before and after each release.
pub trait Probe: Send {
    fn context(&mut self) -> Context;
}

/// The desktop as it is.
#[derive(Debug, Default)]
pub struct Desktop;

impl Probe for Desktop {
    fn context(&mut self) -> Context {
        context()
    }
}

/// Output that keeps the watchdog's table: `B` sends, `P` says what is in
/// front.
pub struct Watched<B = Output, P = Desktop> {
    output: B,
    probe: P,
    table: Arc<Table>,
    /// Whether a release is owed since the last one delivered: only the
    /// first of a run of them is logged.
    owing: bool,
}

impl Watched {
    /// SendInput's output over `table`, wrapped so every press is tracked
    /// and released.
    pub fn open(table: Arc<Table>) -> Input<Watched> {
        Input::new(Watched::over(Output::bare(), Desktop, table))
    }
}

impl<B: InputBackend, P: Probe> Watched<B, P> {
    /// `output`, keeping `table`, with `probe` saying what is in front.
    pub fn over(output: B, probe: P, table: Arc<Table>) -> Watched<B, P> {
        Watched {
            output,
            probe,
            table,
            owing: false,
        }
    }

    fn write(&mut self, item: Held, state: PressState) -> Result<(), InputError> {
        match state {
            PressState::Down => {
                self.table.mark(item);
                send(&mut self.output, item, state)
            }
            PressState::Up => {
                self.release(item);
                Ok(())
            }
        }
    }

    /// Sends `item`'s release and clears its mark if it was delivered;
    /// otherwise it stays marked, as owed.
    fn release(&mut self, item: Held) {
        let before = self.probe.context();
        let sent = send(&mut self.output, item, PressState::Up);
        let after = self.probe.context();
        match judge(before, after, &sent) {
            Delivery::Delivered => {
                self.table.clear(item);
                self.owing = false;
            }
            outcome => {
                if !std::mem::replace(&mut self.owing, true) {
                    let why = sent
                        .err()
                        .map_or_else(String::new, |err| format!(": {err}"));
                    log::line(format_args!(
                        "auto-clicker: a release may not have arrived ({outcome:?}{why}); \
                         it stays owed, and the watchdog sends it when the daemon ends"
                    ));
                }
            }
        }
    }
}

fn send(output: &mut impl InputBackend, item: Held, state: PressState) -> Result<(), InputError> {
    match item {
        Held::Button(button) => output.button(button, state),
        Held::Key(key) => output.key(key, state),
    }
}

impl<B: InputBackend, P: Probe> InputBackend for Watched<B, P> {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        self.write(Held::Button(button), state)
    }

    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError> {
        self.write(Held::Key(key), state)
    }

    fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        self.output.move_relative(dx, dy)
    }

    fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError> {
        self.output.move_absolute(position)
    }

    fn scroll(&mut self, notches: i32) -> Result<(), InputError> {
        self.output.scroll(notches)
    }

    fn run_timer(&mut self) -> Result<Box<dyn RunTimer>, InputError> {
        self.output.run_timer()
    }
}

impl<B, P> std::fmt::Debug for Watched<B, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watched")
            .field("owing", &self.owing)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
