//! Building a recording from what the input reader sees.
//!
//! The reader hands every key, button, motion and wheel event of the devices
//! it reads to the [`Recorder`], with the kernel's timestamp on the
//! monotonic clock. The recorder keeps only what playback can send, in the
//! order it happened:
//!
//! - Motion and wheel are summed over up to a millisecond, so a mouse
//!   reporting 8000 times a second makes no more events than one reporting
//!   1000 times. A key or button event ends the sum first, so it keeps its
//!   place among the moves.
//! - Each device's keys are tracked apart. A key held on two devices is
//!   pressed when the first presses it and released when the last lets go.
//!   A release without its press (a key held before the recording, or one
//!   left out) is dropped. Autorepeat is never recorded.
//! - A hotkey pressed during the recording is left out: its key, and each of
//!   its modifiers pressed at most a second before it with no key or
//!   button recorded since. A modifier held longer (Ctrl held to sprint) is
//!   kept, and released at the end. The recording hotkey also ends the
//!   recording at its own time.
//! - Positions ([`Recorder::position`]) are kept only while the current
//!   layout is the recording's, at most one per millisecond, in time order
//!   among the input events ([`super::timeline`]).
//! - A recording stops by itself before its file would grow past
//!   [`MAX_FILE_BYTES`], as it does at its time limit.
//! - [`Recorder::finish`] trims the trip to the tray on a stop from the tray
//!   menu (see [`super::trim`]), releases everything still down at the end,
//!   and checks the result.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use super::timeline::Timeline;
use super::trim::tray_cut;
use super::{
    Change, Created, Event, InvalidRecording, Layout, MAX_DURATION_US, MAX_FILE_BYTES, Point,
    Recording, Track, codes,
};
use crate::hotkeys::{Action, Binding, Bindings, DeviceId, Modifier};
use crate::input::{Button, Held, Key};

/// How long before a hotkey's key its modifier may have been pressed and
/// still count as part of the hotkey: a second.
const MODIFIER_WINDOW_US: u64 = 1_000_000;

/// How long motion and wheel are summed for.
const BUCKET_US: u64 = 1_000;

/// What a file holds besides its events, at most: the header with the
/// longest name, 16 screens, and a release of every key and button at the
/// end ([`release_all`]). The tests check it.
const FILE_RESERVE: u64 = 64 * 1024;

/// The bytes a recording's events may take in its file.
const EVENT_BUDGET: u64 = MAX_FILE_BYTES - FILE_RESERVE;

/// How a recording ended, as the controller saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Stopped at this time on the event clock: by the hotkey (which the
    /// recorder saw itself, at its own time), a lock, sleep, the time limit
    /// or quitting.
    At(Duration),
    /// Stopped from the tray menu at `at`; `menu_opened` is when the menu
    /// last opened while recording, if the tray said.
    Tray {
        at: Duration,
        menu_opened: Option<Duration>,
    },
}

/// A key or button event, as the reader hands it over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInput {
    pub code: u16,
    /// Went down; `false` when it came up.
    pub pressed: bool,
    /// The hotkey the press fired, if any.
    pub fired: Option<Action>,
}

impl KeyInput {
    pub fn new(code: u16, pressed: bool) -> KeyInput {
        KeyInput {
            code,
            pressed,
            fired: None,
        }
    }

    pub fn press(code: u16) -> KeyInput {
        KeyInput::new(code, true)
    }

    pub fn release(code: u16) -> KeyInput {
        KeyInput::new(code, false)
    }

    /// The same press, having fired `action`.
    pub fn firing(self, action: Action) -> KeyInput {
        KeyInput {
            fired: Some(action),
            ..self
        }
    }
}

/// Motion and wheel summed since `at`.
#[derive(Clone, Copy, Debug)]
struct Bucket {
    at: u64,
    dx: i32,
    dy: i32,
    wheel: i32,
}

impl Bucket {
    /// This bucket with `amount` added at `at`; `None` when `at` is a
    /// millisecond or more after its start, or a sum would overflow.
    fn merged(self, at: u64, amount: Bucket) -> Option<Bucket> {
        if at >= self.at.saturating_add(BUCKET_US) {
            return None;
        }
        Some(Bucket {
            at: self.at,
            dx: self.dx.checked_add(amount.dx)?,
            dy: self.dy.checked_add(amount.dy)?,
            wheel: self.wheel.checked_add(amount.wheel)?,
        })
    }
}

/// A recording in progress.
pub struct Recorder {
    bindings: Bindings,
    /// When the recording started, on the event clock.
    start: Duration,
    created: Created,
    /// The keys of the hotkey that started the recording: on a device's
    /// first read at the start, those it holds are left out. Emptied once
    /// the reader has read every device at the start, so a device added
    /// later records what it holds.
    start_chord: Vec<u16>,
    /// Keys held on a device that are left out until that device lets go
    /// of them: the start hotkey's, and each hotkey's pressed during the
    /// recording. A read after lost events keeps them out too.
    excluded: BTreeSet<(DeviceId, u16)>,
    /// When the recording hotkey stopped the recording.
    stopped_at: Option<u64>,
    /// The layout positions lie on: the current one at the start, if known.
    layout: Option<Layout>,
    timeline: Timeline,
    bucket: Option<Bucket>,
    /// What each device holds.
    devices: BTreeMap<DeviceId, BTreeSet<Held>>,
    /// How many devices hold each key or button that is down.
    down: BTreeMap<Held, u32>,
}

impl Recorder {
    /// A recording starting at `start` on the event clock, made at
    /// `created`. `started_by` is the hotkey that started it, if one did:
    /// its keys, held down at the start, are left out until they come up.
    pub fn new(
        bindings: Bindings,
        start: Duration,
        created: Created,
        started_by: Option<Binding>,
    ) -> Recorder {
        Recorder {
            bindings,
            start,
            created,
            start_chord: started_by.map(chord_codes).unwrap_or_default(),
            excluded: BTreeSet::new(),
            stopped_at: None,
            layout: None,
            timeline: Timeline::new(EVENT_BUDGET),
            bucket: None,
            devices: BTreeMap::new(),
            down: BTreeMap::new(),
        }
    }

    /// The same recording, with positions on `layout`: the current layout
    /// at its start.
    #[must_use]
    pub fn with_layout(self, layout: Layout) -> Recorder {
        Recorder {
            layout: Some(layout),
            ..self
        }
    }

    /// The same recording, whose events may take `budget` bytes in a file.
    #[cfg(test)]
    pub(crate) fn with_budget(self, budget: u64) -> Recorder {
        Recorder {
            timeline: Timeline::new(budget),
            ..self
        }
    }

    /// Whether the recording stopped itself because its file would grow too
    /// large: it takes nothing more, and should be finished and saved.
    pub fn is_full(&self) -> bool {
        self.timeline.is_full()
    }

    /// Whether positions taken on `layout` would be kept: it has the
    /// screens the recording's positions lie on.
    pub fn takes_positions_on(&self, layout: &Layout) -> bool {
        self.layout
            .as_ref()
            .is_some_and(|ours| ours.same_screens(layout))
    }

    /// The pointer at `point`, reported at `time` and taken on `taken_on`
    /// (`None` when unknown): the layout its source reported last before it
    /// (W01 brief, W4). It is kept only when `taken_on` has the recording's
    /// screens ([`Layout::same_screens`]) and the point lies on it.
    pub fn position(&mut self, time: Duration, point: Point, taken_on: Option<&Layout>) {
        let Some(at) = self.offset(time) else {
            return;
        };
        let Some(layout) = &self.layout else {
            return;
        };
        if taken_on.is_some_and(|taken_on| self.takes_positions_on(taken_on)) && layout.holds(point)
        {
            self.timeline.position(at, point);
        }
    }

    /// A key or button event from `device` at `time`.
    pub fn key(&mut self, device: DeviceId, input: KeyInput, time: Duration) {
        let Some(at) = self.offset(time) else {
            return;
        };
        let KeyInput {
            code,
            pressed,
            fired,
        } = input;
        if let (true, Some(action)) = (pressed, fired) {
            self.excluded.insert((device, code));
            return self.hotkey(action, at);
        }
        if self.excluded.contains(&(device, code)) {
            if !pressed {
                self.excluded.remove(&(device, code));
            }
            return;
        }
        let Some(held) = codes::recordable(code) else {
            return;
        };
        if pressed {
            self.press(device, held, at);
        } else {
            self.release(device, held, at);
        }
    }

    /// Relative pointer motion at `time`.
    pub fn motion(&mut self, time: Duration, dx: i32, dy: i32) {
        let moved = dx != 0 || dy != 0;
        if let Some(at) = self.offset(time).filter(|_| moved) {
            self.timeline.motion_read(at);
        }
        self.add(
            time,
            Bucket {
                at: 0,
                dx,
                dy,
                wheel: 0,
            },
        );
    }

    /// Wheel notches at `time`; positive scrolls up.
    pub fn wheel(&mut self, time: Duration, notches: i32) {
        if let Some(at) = self.offset(time)
            && notches != 0
        {
            self.timeline.event_read(at);
        }
        self.add(
            time,
            Bucket {
                at: 0,
                dx: 0,
                dy: 0,
                wheel: notches,
            },
        );
    }

    /// `device` was unplugged or failed at `now`: whatever it held is
    /// released then.
    pub fn device_lost(&mut self, device: DeviceId, now: Duration) {
        let Some(at) = self.offset(now) else {
            return;
        };
        self.excluded.retain(|&(holder, _)| holder != device);
        let held = self.devices.remove(&device).unwrap_or_default();
        for item in held {
            self.release_held(item, at);
        }
    }

    /// What `device` holds at `now`, read from the device: when recording
    /// starts, when a device is added, and after the device lost events.
    /// Keys it no longer holds are released and keys it now holds are
    /// pressed, at `now`.
    pub fn resync(&mut self, device: DeviceId, held: impl Fn(u16) -> bool, now: Duration) {
        let Some(at) = self.offset(now) else {
            return;
        };
        let Some(known) = self.devices.get(&device).cloned() else {
            // The device's first read: the start hotkey's keys it holds are
            // left out.
            let chord = self.start_chord.iter().filter(|&&code| held(code));
            self.excluded.extend(chord.map(|&code| (device, code)));
            self.devices.insert(device, BTreeSet::new());
            return self.press_held(device, &held, at);
        };
        for &item in &known {
            if !held(codes::code(item)) {
                self.release(device, item, at);
            }
        }
        self.excluded
            .retain(|&(holder, code)| holder != device || held(code));
        self.press_held(device, &held, at);
    }

    /// Presses what `device` holds that is not left out.
    fn press_held(&mut self, device: DeviceId, held: &impl Fn(u16) -> bool, at: u64) {
        let pressed: Vec<Held> = Key::CODES
            .chain(Button::ALL.map(Button::code))
            .filter(|&code| held(code) && !self.excluded.contains(&(device, code)))
            .filter_map(codes::recordable)
            .collect();
        for item in pressed {
            self.press(device, item, at);
        }
    }

    /// The reader has read what every device held at the start: devices
    /// read from now on were not part of the start hotkey.
    pub fn start_read(&mut self) {
        self.start_chord.clear();
    }

    /// Ends the recording: trims the trip to the tray after a tray stop,
    /// releases what is still down at the end, and checks the result.
    /// `None` when nothing is left to save.
    ///
    /// # Errors
    /// The recording does not pass [`Recording::new`]'s checks, which would
    /// be a fault in the recorder.
    pub fn finish(mut self, ending: Ending) -> Result<Option<Recording>, InvalidRecording> {
        self.flush();
        let (at, menu_opened) = match ending {
            Ending::At(at) => (at, None),
            Ending::Tray { at, menu_opened } => (at, Some(menu_opened)),
        };
        let stop = match self.timeline.is_full() {
            true => 0,
            false => self.offset(at).unwrap_or(MAX_DURATION_US),
        };
        let mut end = self.stopped_at.unwrap_or(stop).max(self.timeline.last_at());
        let mut events = self.timeline.into_events();
        if let Some(opened) = menu_opened {
            let opened = opened.and_then(|time| offset_from(self.start, time));
            let cut = tray_cut(&events, opened);
            if let Some(first) = events.get(cut) {
                end = first.at;
                events.truncate(cut);
            }
        }
        release_all(&mut events, end);
        let is_position = |event: &Event| matches!(event.change, Change::Position(_));
        if events.iter().all(is_position) {
            return Ok(None);
        }
        let name = self.created.default_name();
        match self.layout.filter(|_| events.iter().any(is_position)) {
            Some(layout) => {
                let track = Track { layout, events };
                Recording::with_positions(name, self.created, end, track)
            }
            None => Recording::new(name, self.created, end, events),
        }
        .map(Some)
    }

    /// Microseconds from the start to `time`; `None` once the recording
    /// stopped, is full, or past its time limit.
    fn offset(&self, time: Duration) -> Option<u64> {
        if self.stopped_at.is_some() || self.timeline.is_full() {
            return None;
        }
        offset_from(self.start, time)
    }

    /// A press that fired `action`: its key is left out, and so are the
    /// modifiers pressed just for it. The recording hotkey stops here.
    fn hotkey(&mut self, action: Action, at: u64) {
        self.flush();
        if let Some((_, binding)) = self
            .bindings
            .actions()
            .into_iter()
            .find(|(bound, _)| *bound == action)
        {
            self.drop_chord_modifiers(binding, at);
        }
        if action == Action::ToggleRecording {
            self.stopped_at = Some(at.max(self.timeline.last_input_at()));
        }
    }

    /// Leaves out each of `binding`'s modifiers that is down, was pressed
    /// at most a second before `at`, and has nothing but motion,
    /// wheel and the binding's other modifiers recorded after its press.
    fn drop_chord_modifiers(&mut self, binding: Binding, at: u64) {
        let chord = modifier_codes(binding);
        let events = self.timeline.events();
        let only_chord_since = |index: usize| {
            events
                .iter()
                .skip(index.saturating_add(1))
                .all(|event| match event.change {
                    Change::Move { .. } | Change::Wheel(_) | Change::Position(_) => true,
                    Change::Press(held) => chord.contains(&codes::code(held)),
                    Change::Release(_) => false,
                })
        };
        let mut dropped: Vec<(usize, Held)> = chord
            .iter()
            .filter_map(|&code| codes::recordable(code))
            .filter(|held| self.down.contains_key(held))
            .filter_map(|held| {
                let index = events
                    .iter()
                    .rposition(|event| event.change == Change::Press(held))?;
                let event = events.get(index)?;
                let recent = at.saturating_sub(event.at) <= MODIFIER_WINDOW_US;
                (recent && only_chord_since(index)).then_some((index, held))
            })
            .collect();
        dropped.sort_unstable_by_key(|&(index, _)| std::cmp::Reverse(index));
        for (index, held) in dropped {
            self.timeline.remove(index);
            self.down.remove(&held);
            for (&device, keys) in &mut self.devices {
                if keys.remove(&held) {
                    self.excluded.insert((device, codes::code(held)));
                }
            }
        }
    }

    fn press(&mut self, device: DeviceId, held: Held, at: u64) {
        if !self.devices.entry(device).or_default().insert(held) {
            return;
        }
        let holders = self.down.entry(held).or_insert(0);
        *holders = holders.saturating_add(1);
        if *holders == 1 {
            self.write(at, Change::Press(held));
        }
    }

    fn release(&mut self, device: DeviceId, held: Held, at: u64) {
        let was_held = self
            .devices
            .get_mut(&device)
            .is_some_and(|keys| keys.remove(&held));
        if was_held {
            self.release_held(held, at);
        }
    }

    /// One device let go of `held`; it comes up once no device holds it.
    fn release_held(&mut self, held: Held, at: u64) {
        let Some(holders) = self.down.get_mut(&held) else {
            return;
        };
        *holders = holders.saturating_sub(1);
        if *holders == 0 {
            self.down.remove(&held);
            self.write(at, Change::Release(held));
        }
    }

    /// Adds motion or wheel to the current bucket, or starts a new one.
    fn add(&mut self, time: Duration, amount: Bucket) {
        let Some(at) = self.offset(time) else {
            return;
        };
        if let Some(merged) = self.bucket.and_then(|bucket| bucket.merged(at, amount)) {
            self.bucket = Some(merged);
            return;
        }
        self.flush();
        self.bucket = Some(Bucket {
            at: at.max(self.timeline.last_input_at()),
            ..amount
        });
    }

    /// Writes the current bucket's motion and wheel, leaving out what sums
    /// to nothing.
    fn flush(&mut self) {
        let Some(bucket) = self.bucket.take() else {
            return;
        };
        if bucket.dx != 0 || bucket.dy != 0 {
            self.write(
                bucket.at,
                Change::Move {
                    dx: bucket.dx,
                    dy: bucket.dy,
                },
            );
        }
        if bucket.wheel != 0 {
            self.write(bucket.at, Change::Wheel(bucket.wheel));
        }
    }

    /// Writes a press or release after the motion before it; no input
    /// event goes before the one written last.
    fn write(&mut self, at: u64, change: Change) {
        if matches!(change, Change::Press(_) | Change::Release(_)) {
            self.flush();
        }
        self.timeline.input(at, change);
    }
}

/// Microseconds from `start` to `time` (0 for a time before the start);
/// `None` past the time limit.
fn offset_from(start: Duration, time: Duration) -> Option<u64> {
    let since = time.saturating_sub(start);
    u64::try_from(since.as_micros())
        .ok()
        .filter(|&micros| micros <= MAX_DURATION_US)
}

/// The key and modifier codes of a hotkey, left and right modifiers both.
fn chord_codes(binding: Binding) -> Vec<u16> {
    let mut codes = modifier_codes(binding);
    codes.push(binding.key().code());
    codes
}

fn modifier_codes(binding: Binding) -> Vec<u16> {
    binding
        .modifiers()
        .iter()
        .flat_map(Modifier::codes)
        .collect()
}

/// Appends, at `end`, a release of everything `events` leave down.
fn release_all(events: &mut Vec<Event>, end: u64) {
    let mut down = BTreeSet::new();
    for event in events.iter() {
        match event.change {
            Change::Press(held) => {
                down.insert(held);
            }
            Change::Release(held) => {
                down.remove(&held);
            }
            Change::Move { .. } | Change::Wheel(_) | Change::Position(_) => {}
        }
    }
    events.extend(down.into_iter().map(|held| Event {
        at: end,
        change: Change::Release(held),
    }));
}

#[cfg(test)]
#[path = "recorder_tests.rs"]
mod tests;
