//! How the watchdog starts the daemon (M5 brief decision 6): the daemon's
//! command line names the two handles it inherits, and nothing else. Shared
//! by the daemon, which reads it, and the watchdog, which writes it.

use std::ffi::OsString;

use super::Handles;

/// The daemon's program, beside the watchdog's.
pub const DAEMON: &str = "auto-clicker.exe";
/// The watchdog's program, beside the daemon's.
pub const WATCHDOG: &str = "auto-clicker-watchdog.exe";

const TABLE: &str = "--table";
const WATCHDOG_HANDLE: &str = "--watchdog";

/// How the daemon was started.
#[derive(Debug, PartialEq, Eq)]
pub enum Launch {
    /// With no arguments at all: it starts the watchdog, which starts it.
    Bare,
    /// By its watchdog, with these handles.
    Watched(Handles),
}

/// The daemon's arguments for `handles`.
pub fn arguments(handles: Handles) -> Vec<OsString> {
    [
        TABLE.to_owned(),
        handles.table.to_string(),
        WATCHDOG_HANDLE.to_owned(),
        handles.watchdog.to_string(),
    ]
    .map(OsString::from)
    .to_vec()
}

/// Reads the daemon's arguments (the program's name left out): none at
/// all, or exactly `--table <n> --watchdog <n>`, each a non-zero decimal
/// handle value. Anything else is refused, saying why.
///
/// # Errors
/// The arguments are neither of those.
pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Launch, String> {
    let arguments: Vec<OsString> = arguments.into_iter().collect();
    if arguments.is_empty() {
        return Ok(Launch::Bare);
    }
    let texts: Vec<&str> = arguments
        .iter()
        .map(|argument| argument.to_str().ok_or("an argument is not text"))
        .collect::<Result<_, _>>()?;
    let [TABLE, table, WATCHDOG_HANDLE, watchdog] = texts[..] else {
        return Err(format!(
            "expected {TABLE} <handle> {WATCHDOG_HANDLE} <handle>, got {}",
            texts.join(" ")
        ));
    };
    Ok(Launch::Watched(Handles {
        table: handle(TABLE, table)?,
        watchdog: handle(WATCHDOG_HANDLE, watchdog)?,
    }))
}

/// A handle's value, as `name` gives it: decimal digits alone, not 0.
fn handle(name: &str, value: &str) -> Result<usize, String> {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| value.parse::<usize>().ok())
        .flatten()
        .filter(|&value| value != 0)
        .ok_or_else(|| format!("{name} needs a handle's value, not {value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(arguments: &[&str]) -> Result<Launch, String> {
        parse(arguments.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_is_a_bare_start_and_the_watchdogs_are_read_back() {
        assert_eq!(read(&[]), Ok(Launch::Bare));
        let handles = Handles {
            table: 0x1a4,
            watchdog: 12,
        };
        assert_eq!(parse(arguments(handles)), Ok(Launch::Watched(handles)));
    }

    #[test]
    fn anything_else_is_refused_never_a_bare_start() {
        for refused in [
            &["--table"][..],
            &["--table", "4"],
            &["--table", "4", "--watchdog"],
            &["--watchdog", "8", "--table", "4"],
            &["--table", "4", "--watchdog", "8", "--table", "4"],
            &["--table", "0", "--watchdog", "8"],
            &["--table", "+4", "--watchdog", "8"],
            &["--table", "0x4", "--watchdog", "8"],
            &["--table", "4", "--watchdog", "-8"],
            &["--table", "4", "--watchdog", "99999999999999999999999"],
            &["--table", "", "--watchdog", "8"],
            &[""],
            &["--quit"],
        ] {
            assert!(read(refused).is_err(), "{refused:?}");
        }
    }
}
