//! The token baked into each load of a script: a call is accepted only with
//! the token of a load this daemon made, so a script a crashed daemon left
//! behind, or an older load, is not heard. It guards against stale scripts,
//! not against other processes of the same user (brief decision 5).

use std::fmt;

/// 128 random bits as 32 hex digits.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct Token(String);

impl Token {
    /// A new token from `rng`, which is seeded at random once per daemon,
    /// so tokens differ across daemon runs.
    pub(crate) fn new(rng: &mut fastrand::Rng) -> Token {
        Token(format!("{:016x}{:016x}", rng.u64(..), rng.u64(..)))
    }

    /// The token as it was sent, if it has the shape of one; anything else
    /// is no token of ours.
    pub(crate) fn parse(text: &str) -> Option<Token> {
        let hex = text.len() == 32 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        hex.then(|| Token(text.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Shortened: a token is no secret, but logs need only tell two apart.
impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Token({}…)", self.0.get(..8).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_32_hex_digits_and_differ() {
        let mut rng = fastrand::Rng::with_seed(7);
        let first = Token::new(&mut rng);
        let second = Token::new(&mut rng);
        assert_ne!(first, second);
        assert_eq!(Token::parse(first.as_str()), Some(first.clone()));
        assert_eq!(format!("{first:?}").len(), "Token(…)".len() + 8);
        for refused in ["", "abc", &"A".repeat(32), &"g".repeat(32), &"0".repeat(33)] {
            assert_eq!(Token::parse(refused), None, "{refused:?}");
        }
    }
}
