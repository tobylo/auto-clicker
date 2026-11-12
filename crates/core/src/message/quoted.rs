//! Text from outside the program, quoted as it is inside a message.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::window::{MAX_TEXT, bounded};

/// Text another program or the operating system wrote: an I/O error, a
/// portal's reason, a parser's message. A message quotes it as it is, in
/// whatever language the message is shown in.
///
/// It goes to the settings window cut to [`MAX_TEXT`] characters, and the
/// window takes none longer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quoted(String);

impl Quoted {
    pub fn new(text: impl Into<String>) -> Quoted {
        Quoted(text.into())
    }
}

impl From<String> for Quoted {
    fn from(text: String) -> Quoted {
        Quoted(text)
    }
}

impl From<&str> for Quoted {
    fn from(text: &str) -> Quoted {
        Quoted(text.to_owned())
    }
}

impl std::ops::Deref for Quoted {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Quoted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Quoted {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&bounded(&self.0))
    }
}

impl<'de> Deserialize<'de> for Quoted {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Quoted, D::Error> {
        let text = String::deserialize(deserializer)?;
        // Only a cut text is longer than the limit: by the "…" the cut adds.
        let mut over = text.chars().skip(MAX_TEXT);
        let fits = match (over.next(), over.next()) {
            (None, _) => true,
            (Some(last), None) => last == '…',
            (Some(_), Some(_)) => false,
        };
        if !fits {
            return Err(serde::de::Error::custom(format!(
                "a quoted text is longer than {MAX_TEXT} characters"
            )));
        }
        Ok(Quoted(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_goes_out_cut_and_comes_back_as_it_went() {
        let long = Quoted::new("é".repeat(MAX_TEXT * 2));
        let line = serde_json::to_string(&long).unwrap();
        let back: Quoted = serde_json::from_str(&line).unwrap();
        assert_eq!(&*back, bounded(&long));
        let short = Quoted::new("disk full");
        assert_eq!(&*short, "disk full");
        let back: Quoted = serde_json::from_str(&serde_json::to_string(&short).unwrap()).unwrap();
        assert_eq!(back, short);
    }

    #[test]
    fn a_longer_text_than_a_cut_one_is_refused() {
        let at_most = format!("\"{}\"", "a".repeat(MAX_TEXT));
        assert!(serde_json::from_str::<Quoted>(&at_most).is_ok());
        let cut = format!("\"{}…\"", "a".repeat(MAX_TEXT));
        assert!(serde_json::from_str::<Quoted>(&cut).is_ok());
        let uncut = format!("\"{}\"", "a".repeat(MAX_TEXT + 1));
        assert!(serde_json::from_str::<Quoted>(&uncut).is_err());
        let over = format!("\"{}\"", "a".repeat(MAX_TEXT + 2));
        assert!(serde_json::from_str::<Quoted>(&over).is_err());
    }
}
