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
//! A recording with positions is version 2: a `layout` field after
//! `duration_us`, one object per screen, and `"at"` events.
//!
//! ```json
//! {"version":2,"name":"Recording 2026-11-09 18:04","created":"2026-11-09T18:04:05.123+01:00","duration_us":52000,"layout":[{"x":0,"y":0,"width":2048,"height":1280,"scale_milli":1250}],"events":[
//! [0,"at",640,400],
//! [1200,"move",3,-2],
//! [1900,"at",643,398],
//! [52000,"press","BTN_LEFT"],
//! [52000,"release","BTN_LEFT"]
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

use super::{
    Change, Created, Event, InvalidRecording, Layout, MAX_DURATION_US, Output, Point, Recording,
    Rect, Track, VERSION, VERSION_POSITIONS, codes,
};

impl Recording {
    /// Writes the recording, one event per line: as version 2 when it has
    /// positions, as version 1 otherwise.
    ///
    /// # Errors
    /// Only what `out` returns.
    pub fn write_json(&self, out: &mut dyn Write) -> io::Result<()> {
        let version = match self.layout {
            Some(_) => VERSION_POSITIONS,
            None => VERSION,
        };
        write!(out, "{{\"version\":{version},\"name\":")?;
        serde_json::to_writer(&mut *out, &self.name)?;
        write!(
            out,
            ",\"created\":\"{}\",\"duration_us\":{}",
            self.created, self.duration_us
        )?;
        if let Some(layout) = &self.layout {
            write_layout(out, layout)?;
        }
        out.write_all(b",\"events\":[")?;
        for (index, event) in self.events.iter().enumerate() {
            let separator = if index == 0 { "\n" } else { ",\n" };
            out.write_all(separator.as_bytes())?;
            write_event(out, *event)?;
        }
        out.write_all(b"\n]}\n")
    }

    /// Reads a recording file.
    ///
    /// # Errors
    /// The file is not JSON, does not have a recording's layout, has a
    /// version other than 1 or 2, is a version 1 file with a layout or a
    /// position, a version 2 file without a layout, or holds a recording
    /// [`Recording::new`] or [`Recording::with_positions`] refuses.
    pub fn from_json(bytes: &[u8]) -> Result<Recording, InvalidRecording> {
        let header: Header = serde_json::from_slice(bytes).map_err(malformed)?;
        if header.version != VERSION && header.version != VERSION_POSITIONS {
            return Err(InvalidRecording::new(format!(
                "the file is a version {} recording; this program reads versions \
                 {VERSION} and {VERSION_POSITIONS}",
                header.version
            )));
        }
        let file: File = serde_json::from_slice(bytes).map_err(malformed)?;
        let layout = file_layout(header.version, file.layout)?;
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
        match layout {
            Some(layout) => {
                let track = Track { layout, events };
                Recording::with_positions(file.name, created, file.duration_us, track)
            }
            None => {
                let position = events
                    .iter()
                    .position(|event| matches!(event.change, Change::Position(_)));
                if let Some(index) = position {
                    return Err(InvalidRecording::at(
                        index,
                        "is a position, which a version 1 recording cannot hold",
                    ));
                }
                Recording::new(file.name, created, file.duration_us, events)
            }
        }
    }
}

/// How many bytes `event` takes in a file at most, its separator included:
/// with the longest time a recording has, so an event moved later never
/// takes more.
pub fn event_bytes(change: Change) -> u64 {
    let mut counter = Counter(0);
    let event = Event {
        at: MAX_DURATION_US,
        change,
    };
    // A key or button without a name is never written; it counts nothing.
    if write_event(&mut counter, event).is_err() {
        return 0;
    }
    counter.0.saturating_add(2)
}

/// Counts what is written to it.
struct Counter(u64);

impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let added = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.0 = self.0.saturating_add(added);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The layout version `version` asks for: none in version 1, one in
/// version 2.
fn file_layout(
    version: u64,
    outputs: Option<Vec<RawOutput>>,
) -> Result<Option<Layout>, InvalidRecording> {
    match (version, outputs) {
        (VERSION, None) => Ok(None),
        (VERSION, Some(_)) => Err(InvalidRecording::new(format!(
            "a version {VERSION} recording has no layout"
        ))),
        (_, None) => Err(InvalidRecording::new(format!(
            "a version {VERSION_POSITIONS} recording needs a layout"
        ))),
        (_, Some(outputs)) => {
            let outputs = outputs
                .into_iter()
                .map(|raw| {
                    let rect = Rect {
                        x: raw.x,
                        y: raw.y,
                        width: raw.width,
                        height: raw.height,
                    };
                    Output::new(rect, raw.scale_milli)
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(InvalidRecording::new)?;
            Layout::new(outputs)
                .map(Some)
                .map_err(InvalidRecording::new)
        }
    }
}

fn write_layout(out: &mut dyn Write, layout: &Layout) -> io::Result<()> {
    out.write_all(b",\"layout\":[")?;
    for (index, output) in layout.outputs().iter().enumerate() {
        let separator = if index == 0 { "" } else { "," };
        let Rect {
            x,
            y,
            width,
            height,
        } = output.rect();
        let scale = output.scale_milli();
        write!(
            out,
            "{separator}{{\"x\":{x},\"y\":{y},\"width\":{width},\"height\":{height},\"scale_milli\":{scale}}}"
        )?;
    }
    out.write_all(b"]")
}

fn write_event(out: &mut dyn Write, event: Event) -> io::Result<()> {
    write!(out, "[{},", event.at)?;
    write_change(out, event.change)?;
    out.write_all(b"]")
}

fn write_change(out: &mut dyn Write, change: Change) -> io::Result<()> {
    let name =
        |held| codes::name(held).ok_or_else(|| io::Error::other("a key or button without a name"));
    match change {
        Change::Press(held) => write!(out, "\"press\",\"{}\"", name(held)?),
        Change::Release(held) => write!(out, "\"release\",\"{}\"", name(held)?),
        Change::Move { dx, dy } => write!(out, "\"move\",{dx},{dy}"),
        Change::Wheel(notches) => write!(out, "\"wheel\",{notches}"),
        Change::Position(Point { x, y }) => write!(out, "\"at\",{x},{y}"),
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
    #[serde(default, deserialize_with = "present")]
    layout: Option<Vec<RawOutput>>,
    events: Vec<RawEvent>,
}

/// A field that is there holds its value: `null` is not a layout.
fn present<'de, D: de::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<RawOutput>>, D::Error> {
    Vec::deserialize(deserializer).map(Some)
}

/// One screen as the file has it, checked by [`Output::new`].
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOutput {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    scale_milli: u32,
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
            Some(Kind::At) => Some(Change::Position(Point {
                x: next(&mut seq)?,
                y: next(&mut seq)?,
            })),
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
    At,
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
            "at" => Some(Kind::At),
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
