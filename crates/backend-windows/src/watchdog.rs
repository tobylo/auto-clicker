//! The watchdog's side of M5 brief decision 6, and what the daemon takes on
//! from it: the shared table of what is held ([`Table`], [`Owed`]), the two
//! handles that pass by inheritance ([`start_daemon`], [`attach`]) and the
//! daemon's command line that names them ([`arguments`], [`parse`]), and the
//! releases the watchdog sends once its daemon has ended ([`release`]).

mod handles;
mod launch;
mod table;

use auto_clicker_core::input::{Held, InputBackend, PressState};

pub use handles::{Attached, Daemon, Handles, attach, start_daemon};
pub use launch::{DAEMON, Launch, WATCHDOG, arguments, parse};
pub use table::{Owed, Section, TABLE_LEN, Table};

use crate::delivery::{Delivery, context, judge};
use crate::output::Output;

/// Sends each of `owed`'s releases once, each judged by brief decision 7,
/// and says what became of each. A release that may not have arrived is
/// sent once only: sending it again is WN06b's.
pub fn release(owed: &Owed) -> Vec<(Held, Delivery)> {
    let mut output = Output::bare();
    owed.items()
        .iter()
        .map(|&item| {
            let before = context();
            let sent = match item {
                Held::Button(button) => output.button(button, PressState::Up),
                Held::Key(key) => output.key(key, PressState::Up),
            };
            (item, judge(before, context(), &sent))
        })
        .collect()
}
