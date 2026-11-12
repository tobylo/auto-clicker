//! A recording as a message names it.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::Quoted;
use crate::recording::FileName;

/// A recording, by its file and the name in its header, if that read: the
/// lists call it by when it was made when it has the name a new recording
/// gets, else by its name and when.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Titled {
    pub file: FileName,
    /// The name the user gave it, as it is.
    pub name: Option<Quoted>,
}

impl Titled {
    pub fn new(file: FileName, name: Option<&str>) -> Titled {
        Titled {
            file,
            name: name.map(Quoted::from),
        }
    }

    /// Whether the lists call it by when it was made alone: it has the
    /// name a new recording gets, or none could be read. A renderer words
    /// that in its own language; a name the user gave stays as it is.
    pub fn generated(&self) -> bool {
        self.to_string() == self.file.label()
    }
}

/// What the lists call it in English: [`FileName::titled`].
impl fmt::Display for Titled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.file.titled(self.name.as_deref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> FileName {
        FileName::parse("2026-11-09_18-04-05.123.json").unwrap()
    }

    #[test]
    fn only_the_name_a_new_recording_gets_is_generated() {
        let cases = [
            (None, true, "Recording 2026-11-09 18:04:05"),
            (
                Some("Recording 2026-11-09 18:04"),
                true,
                "Recording 2026-11-09 18:04:05",
            ),
            (
                Some("Recording practice"),
                false,
                "Recording practice (2026-11-09 18:04:05)",
            ),
        ];
        for (name, generated, english) in cases {
            let titled = Titled::new(file(), name);
            assert_eq!(titled.generated(), generated, "{name:?}");
            assert_eq!(titled.to_string(), english);
        }
    }
}
