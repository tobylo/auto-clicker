//! Whether the user's session lets clicking start: the facts logind reports,
//! folded into one answer.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Why clicking may not run now.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Blocked {
    /// The screen is locked.
    Locked,
    /// Another session is in front: a user switch, or a text console.
    Inactive,
    /// The computer is about to sleep.
    Sleeping,
    /// The session state could not be read; the reason. A state that cannot
    /// be read is never taken for "unlocked".
    Unknown(String),
}

impl fmt::Display for Blocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Blocked::Locked => f.write_str("the screen is locked"),
            Blocked::Inactive => f.write_str("another session is in front"),
            Blocked::Sleeping => f.write_str("the computer is going to sleep"),
            Blocked::Unknown(reason) => write!(f, "the session state is unknown: {reason}"),
        }
    }
}

/// What logind last said about the user's graphical session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionFacts {
    /// The session's `LockedHint`.
    pub locked: bool,
    /// The session's `Active`.
    pub active: bool,
    /// Between `PrepareForSleep(true)` and `PrepareForSleep(false)`.
    pub sleeping: bool,
}

impl SessionFacts {
    /// Why clicking may not run, or `None` when it may. Sleep comes first,
    /// then the lock, then another session in front.
    pub fn blocked(self) -> Option<Blocked> {
        if self.sleeping {
            Some(Blocked::Sleeping)
        } else if self.locked {
            Some(Blocked::Locked)
        } else if !self.active {
            Some(Blocked::Inactive)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_active_unlocked_awake_session_may_click() {
        let open = SessionFacts {
            locked: false,
            active: true,
            sleeping: false,
        };
        assert_eq!(open.blocked(), None);
    }

    #[test]
    fn each_fact_blocks_in_order() {
        // (sleeping, locked, inactive)
        let cases = [
            ((true, true, true), Blocked::Sleeping),
            ((true, false, false), Blocked::Sleeping),
            ((false, true, true), Blocked::Locked),
            ((false, true, false), Blocked::Locked),
            ((false, false, true), Blocked::Inactive),
        ];
        for ((sleeping, locked, inactive), expected) in cases {
            let facts = SessionFacts {
                locked,
                active: !inactive,
                sleeping,
            };
            assert_eq!(facts.blocked(), Some(expected), "{facts:?}");
        }
    }

    #[test]
    fn unknown_names_its_reason() {
        let unknown = Blocked::Unknown("no graphical session".to_owned());
        assert_eq!(
            unknown.to_string(),
            "the session state is unknown: no graphical session"
        );
    }
}
