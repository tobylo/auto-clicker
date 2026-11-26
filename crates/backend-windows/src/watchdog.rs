//! The watchdog's side of M5 brief decision 6, and what the daemon takes on
//! from it: the shared table of what is held ([`Table`], [`Owed`]), the two
//! handles that pass by inheritance ([`start_daemon`], [`attach`]) and the
//! daemon's command line that names them ([`arguments`], [`parse`]), the
//! releases the watchdog sends once its daemon has ended ([`release`]), and
//! the daemon's wait on its watchdog ([`wait_for_watchdog`]).

mod handles;
mod launch;
mod table;
mod wait;

use auto_clicker_core::input::{Held, InputBackend, PressState};

pub use handles::{Attached, Daemon, Handles, attach, start_daemon};
pub use launch::{DAEMON, Launch, WATCHDOG, arguments, parse};
pub use table::{Owed, Section, TABLE_LEN, Table};
pub use wait::{WatchdogWait, wait_for_watchdog};

use crate::delivery::{Delivery, judge};
use crate::output::Output;
use crate::watched::{Desktop, Probe};

/// Sends each of `owed`'s releases once, each judged by brief decision 7,
/// and says what became of each. A release that may not have arrived is
/// sent once only: sending it again is WN06b's.
pub fn release(owed: &Owed) -> Vec<(Held, Delivery)> {
    release_through(owed, &mut Output::bare(), &mut Desktop)
}

/// [`release`], through `output`, with `probe` saying what is in front.
fn release_through(
    owed: &Owed,
    output: &mut impl InputBackend,
    probe: &mut impl Probe,
) -> Vec<(Held, Delivery)> {
    owed.items()
        .iter()
        .map(|&item| {
            let before = probe.context();
            let sent = match item {
                Held::Button(button) => output.button(button, PressState::Up),
                Held::Key(key) => output.key(key, PressState::Up),
            };
            (item, judge(before, probe.context(), &sent))
        })
        .collect()
}

#[cfg(test)]
mod tests;
