//! Why a config file could not be used.

use std::fmt;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::message::ConfigProblem;

/// A config file that could not be used, and why. The program runs on the
/// defaults meanwhile; the tray shows this message.
#[derive(Debug)]
pub struct ConfigError {
    path: PathBuf,
    problem: Problem,
}

/// What is wrong with a config file.
#[derive(Debug)]
pub enum Problem {
    /// The file exists but could not be read (permission denied, a
    /// directory, an I/O error).
    Read(io::Error),
    /// The file's contents are not a valid config. `line` counts from 1, and
    /// is `None` when the problem is not at one place.
    Invalid { line: Option<usize>, reason: String },
}

impl Problem {
    /// A problem at byte `span` of `text`.
    pub(super) fn invalid(text: &str, span: Option<Range<usize>>, reason: String) -> Problem {
        Problem::Invalid {
            line: span.map(|span| line_of(text, span.start)),
            reason,
        }
    }
}

/// The line, from 1, that byte `offset` of `text` is on.
fn line_of(text: &str, offset: usize) -> usize {
    let before = text.as_bytes().get(..offset).unwrap_or(text.as_bytes());
    before
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        .saturating_add(1)
}

impl ConfigError {
    pub(super) fn new(path: &Path, problem: Problem) -> ConfigError {
        ConfigError {
            path: path.to_owned(),
            problem,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn problem(&self) -> &Problem {
        &self.problem
    }
}

/// The same words the tray shows, in English: see [`ConfigProblem`].
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        ConfigProblem::from(self).fmt(f)
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.problem {
            Problem::Read(err) => Some(err),
            Problem::Invalid { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_count_from_one() {
        let text = "a\nb\nc";
        assert_eq!(line_of(text, 0), 1);
        assert_eq!(line_of(text, 2), 2);
        assert_eq!(line_of(text, 4), 3);
        assert_eq!(line_of(text, 99), 3);
    }

    #[test]
    fn messages_name_the_file_and_line() {
        let path = Path::new("/home/me/.config/auto-clicker/config.toml");
        let invalid = ConfigError::new(
            path,
            Problem::Invalid {
                line: Some(4),
                reason: "bad".to_owned(),
            },
        );
        let unreadable =
            ConfigError::new(path, Problem::Read(io::ErrorKind::PermissionDenied.into()));
        assert_eq!(
            invalid.to_string(),
            "/home/me/.config/auto-clicker/config.toml:4: bad"
        );
        assert!(
            unreadable.to_string().starts_with(
                "/home/me/.config/auto-clicker/config.toml: could not read the file: "
            )
        );
    }

    #[test]
    fn only_a_read_error_has_a_source() {
        use std::error::Error;
        let path = Path::new("config.toml");
        let unreadable = ConfigError::new(path, Problem::Read(io::Error::other("disk")));
        let invalid = ConfigError::new(
            path,
            Problem::Invalid {
                line: None,
                reason: "bad".to_owned(),
            },
        );
        let sources = (
            unreadable.source().map(ToString::to_string),
            invalid.source().is_none(),
        );
        assert_eq!(sources, (Some("disk".to_owned()), true));
    }
}
