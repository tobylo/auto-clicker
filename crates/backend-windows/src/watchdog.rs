//! The watchdog's side of M5 brief decision 6, and what the daemon takes on
//! from it: the shared table of what is held ([`Table`], [`Owed`]), the two
//! handles that pass by inheritance ([`start_daemon`], [`attach`]) and the
//! daemon's command line that names them ([`arguments`], [`parse`]), the
//! releases the watchdog sends once its daemon has ended and again until
//! they arrive ([`release`], [`Released::send_until_delivered`]), the
//! watchdogs' mutex a new daemon waits on ([`earlier_watchdog`]), and the
//! daemon's wait on its watchdog ([`wait_for_watchdog`]).

mod after;
mod handles;
mod launch;
mod mutex;
mod table;
mod wait;

pub use after::{Released, Stayed, release};
pub use handles::{Attached, Daemon, Handles, attach, start_daemon};
pub use launch::{DAEMON, Launch, WATCHDOG, arguments, parse};
pub use mutex::{Earlier, EarlierWait, earlier_watchdog};
pub use table::{Owed, Section, TABLE_LEN, Table};
pub use wait::{WatchdogWait, wait_for_watchdog};
