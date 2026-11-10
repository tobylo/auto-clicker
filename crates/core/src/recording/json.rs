//! Recording files: JSON with a header and one compact array per event.
//!
//! ```json
//! {"version":1,"name":"Recording 2026-11-09 18:04","created":"2026-11-09T18:04:05.123+01:00","duration_us":2500000,"events":[
//! [0,"press","KEY_W"],
//! [1200,"move",3,-2],
//! [480000,"wheel",-1],
//! [2500000,"release","KEY_W"]
//! ]}
//! ```
//!
//! Times are microseconds since the start. Keys and buttons go by their
//! kernel names ([`super::codes`]). A file is parsed whole, then checked by
//! [`Recording::new`] like any recording; errors give a place in the file,
//! never the text found there, which could be a key name.

use std::fmt;
use std::io::{self, Write};

use serde::Deserialize;
use serde::de::{self, IgnoredAny, SeqAccess, Visitor};
use serde_json::error::Category;

use super::{Change, Created, Event, InvalidRecording, Recording, VERSION, codes};

impl Recording {
    /// Writes the recording as a version 1 file, one event per line.
    ///
    /// # Errors
    /// Only what `out` returns.
    pub fn write_json(&self, out: &mut dyn Write) -> io::Result<()> {
        write!(out, "{{\"version\":{VERSION},\"name\":")?;
        serde_json::to_writer(&mut *out, &self.name)?;
        write!(
            out,
            ",\"created\":\"{}\",\"duration_us\":{},\"events\":[",
            self.created, self.duration_us
        )?;
        for (index, event) in self.events.iter().enumerate() {
            let separator = if index == 0 { "\n" } else { ",\n" };
            write!(out, "{separator}[{},", event.at)?;
            write_change(out, event.change)?;
            out.write_all(b"]")?;
        }
        out.write_all(b"\n]}\n")
    }

    /// Reads a recording file.
    ///
    /// # Errors
    /// The file is not JSON, does not have a recording's layout, has a
    /// version other than 1, or holds a recording [`Recording::new`]
    /// refuses.
    pub fn from_json(bytes: &[u8]) -> Result<Recording, InvalidRecording> {
        let header: Header = serde_json::from_slice(bytes).map_err(malformed)?;
        if header.version != VERSION {
            return Err(InvalidRecording::new(format!(
                "the file is a version {} recording; this program reads version {VERSION}",
                header.version
            )));
        }
        let file: File = serde_json::from_slice(bytes).map_err(malformed)?;
        let created = Created::parse(&file.created)
            .ok_or_else(|| InvalidRecording::new("the recording's creation time is malformed"))?;
        let events = file
            .events
            .into_iter()
            .enumerate()
            .map(|(index, event)| {
                event
                    .change
                    .map(|change| Event {
                        at: event.at,
                        change,
                    })
                    .ok_or_else(|| {
                        InvalidRecording::at(index, "names an unknown kind, key or button")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Recording::new(file.name, created, file.duration_us, events)
    }
}

fn write_change(out: &mut dyn Write, change: Change) -> io::Result<()> {
    let name =
        |held| codes::name(held).ok_or_else(|| io::Error::other("a key or button without a name"));
    match change {
        Change::Press(held) => write!(out, "\"press\",\"{}\"", name(held)?),
        Change::Release(held) => write!(out, "\"release\",\"{}\"", name(held)?),
        Change::Move { dx, dy } => write!(out, "\"move\",{dx},{dy}"),
        Change::Wheel(notches) => write!(out, "\"wheel\",{notches}"),
    }
}

/// Where a file failed to parse, without what was there.
fn malformed(err: serde_json::Error) -> InvalidRecording {
    let what = match err.classify() {
        Category::Io => "the file could not be read",
        Category::Syntax => "the file is not valid JSON",
        Category::Data => "the file does not have a recording's layout",
        Category::Eof => "the file ends too early",
    };
    InvalidRecording::new(format!(
        "{what} (line {}, column {})",
        err.line(),
        err.column()
    ))
}

/// The version alone, read first: a newer file may differ in everything else.
#[derive(Deserialize)]
struct Header {
    version: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(rename = "version")]
    _version: u64,
    name: String,
    created: String,
    duration_us: u64,
    events: Vec<RawEvent>,
}

/// One event as the file has it; `change` is `None` for an unknown kind or
/// name, which [`Recording::from_json`] reports with the event's place.
struct RawEvent {
    at: u64,
    change: Option<Change>,
}

impl<'de> Deserialize<'de> for RawEvent {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<RawEvent, D::Error> {
        deserializer.deserialize_seq(EventVisitor)
    }
}

struct EventVisitor;

impl<'de> Visitor<'de> for EventVisitor {
    type Value = RawEvent;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an event: [time, kind, values...]")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<RawEvent, A::Error> {
        let at: u64 = next(&mut seq)?;
        let change = match next::<Word, _>(&mut seq)?.0 {
            Some(Kind::Press) => next::<Name, _>(&mut seq)?.0.map(Change::Press),
            Some(Kind::Release) => next::<Name, _>(&mut seq)?.0.map(Change::Release),
            Some(Kind::Move) => Some(Change::Move {
                dx: next(&mut seq)?,
                dy: next(&mut seq)?,
            }),
            Some(Kind::Wheel) => Some(Change::Wheel(next(&mut seq)?)),
            None => None,
        };
        // An unknown kind's values are skipped; it is refused anyway.
        if change.is_none() {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
        } else if seq.next_element::<IgnoredAny>()?.is_some() {
            return Err(de::Error::custom("an event has too many values"));
        }
        Ok(RawEvent { at, change })
    }
}

fn next<'de, T: Deserialize<'de>, A: SeqAccess<'de>>(seq: &mut A) -> Result<T, A::Error> {
    seq.next_element()?
        .ok_or_else(|| de::Error::custom("an event has too few values"))
}

/// An event's kind; `None` when unknown.
struct Word(Option<Kind>);

enum Kind {
    Press,
    Release,
    Move,
    Wheel,
}

/// A key or button name; `None` when unknown.
struct Name(Option<crate::input::Held>);

/// Reads a string without keeping it, mapping it through `map`.
struct StrVisitor<F>(F);

impl<'de, T, F: FnOnce(&str) -> T> Visitor<'de> for StrVisitor<F> {
    type Value = T;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a string")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<T, E> {
        Ok((self.0)(text))
    }
}

impl<'de> Deserialize<'de> for Word {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Word, D::Error> {
        let kind = |text: &str| match text {
            "press" => Some(Kind::Press),
            "release" => Some(Kind::Release),
            "move" => Some(Kind::Move),
            "wheel" => Some(Kind::Wheel),
            _ => None,
        };
        deserializer.deserialize_str(StrVisitor(kind)).map(Word)
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Name, D::Error> {
        deserializer
            .deserialize_str(StrVisitor(codes::parse))
            .map(Name)
    }
}
