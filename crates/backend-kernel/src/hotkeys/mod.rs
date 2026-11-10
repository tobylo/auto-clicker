//! The input reader on the kernel backend: global hotkeys from every
//! keyboard in `/dev/input`, and while recording, what every mouse and
//! keyboard does, all read without grabbing anything.
//!
//! [`read_input`] opens each node with a bound key or a modifier key, except
//! this program's own virtual devices, and watches the directory for devices
//! coming and going. Key events go to one [`Matcher`] for all keyboards;
//! what it says goes to the controller. While a recording runs, it also
//! opens mice and the other keyboards, and hands what each recordable device
//! does to the recorder. Input reaches the game too: nothing is grabbed.

mod events;
mod node;
mod recording;
mod registry;
mod watch;

use std::ffi::{OsStr, OsString};
use std::future::{Future, poll_fn};
use std::io;
use std::path::PathBuf;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use auto_clicker_core::hotkeys::{
    Bindings, DeviceId, HotkeyEvent, HotkeyStatus, Matcher, Reaction, Unavailable,
};
use auto_clicker_core::recording::{KeyInput, Recorder, RecordingFeed};
use evdev::raw_stream::EventStream;
use evdev::{AttributeSet, InputEvent, KeyCode};
use tokio::task::coop;

use crate::DeviceIdentity;
use crate::clock::monotonic_now;
use events::Step;
use node::{Node, Paths, Role, Wanted};
use registry::{Opened, Registry};
use watch::{Change, Watch};

/// Reads every keyboard for hotkeys until `shutdown` resolves, and hands
/// what happens to `send`: fired bindings, the release after each, and
/// whether hotkeys are available. While a recording runs in `feed`'s slot,
/// it also reads every mouse and keyboard, and feeds the recorder.
///
/// When a recording starts, it reads what every device holds then, so a
/// key pressed while the controller was starting the recording is recorded
/// as held from the start.
///
/// `own` must hold every virtual device this program has open, and be
/// complete before this starts: a device created later would be read like
/// any other. A backend that is recreated needs this restarted with the new
/// identities.
///
/// Runs inside a tokio runtime with I/O enabled. `send` must not block.
/// When watching `/dev/input` fails, it reports
/// [`Unavailable::WatchFailed`] and returns.
pub async fn read_input(
    sources: InputSources<'_>,
    send: impl FnMut(HotkeyEvent),
    shutdown: impl Future<Output = ()>,
) {
    let setup = Setup {
        bindings: sources.bindings,
        own: sources
            .own
            .iter()
            .map(|device| device.syspath.clone())
            .collect(),
        feed: sources.feed,
    };
    run(Reader::new(Paths::system(), setup, send), shutdown).await;
}

/// What [`read_input`] reads for, and what it leaves alone.
pub struct InputSources<'a> {
    /// The hotkeys to match.
    pub bindings: Bindings,
    /// Every virtual device this program has open, never read.
    pub own: &'a [DeviceIdentity],
    /// Where a recording in progress is fed.
    pub feed: RecordingFeed,
}

/// The reader's setup, with its own devices by sysfs path.
struct Setup {
    bindings: Bindings,
    own: Vec<PathBuf>,
    feed: RecordingFeed,
}

/// What woke the reader.
enum Wake {
    ShutDown,
    Recording,
    Changes(io::Result<Vec<Change>>),
    Event(usize, io::Result<InputEvent>),
}

async fn run<S: FnMut(HotkeyEvent)>(mut reader: Reader<S>, shutdown: impl Future<Output = ()>) {
    let mut shutdown = pin!(shutdown);
    // The watch starts before the scan, so no node appears unseen between.
    let mut watch = match Watch::new(&reader.paths.dev) {
        Ok(watch) => watch,
        Err(err) => return reader.stop(&err),
    };
    // A recording already running when the reader starts is caught up
    // with at once.
    let started = if reader.feed.recording() {
        reader.recording_changed()
    } else {
        reader.scan()
    };
    if let Err(err) = started {
        return reader.stop(&err);
    }
    loop {
        reader.report();
        let wake = tokio::select! {
            biased;
            () = &mut shutdown => Wake::ShutDown,
            () = reader.feed.changed() => Wake::Recording,
            changes = watch.changes() => Wake::Changes(changes),
            (index, event) = next_event(&mut reader.devices) => Wake::Event(index, event),
        };
        let handled = match wake {
            Wake::ShutDown => return,
            Wake::Recording => reader.recording_changed(),
            Wake::Changes(changes) => changes.and_then(|changes| reader.changed(changes)),
            Wake::Event(index, event) => {
                reader.event(index, event);
                Ok(())
            }
        };
        if let Err(err) = handled {
            return reader.stop(&err);
        }
    }
}

/// The next event from any keyboard, and which one sent it: of the events
/// waiting, the earliest by kernel timestamp, so Ctrl on one device and F6
/// on another keep their order however late the reader looks. A report of
/// lost events comes before any other, since events lost before it may be
/// older than the others waiting; an error is returned at once. Never
/// resolves while no event waits.
fn next_event(
    devices: &mut [Reading],
) -> impl Future<Output = (usize, io::Result<InputEvent>)> + '_ {
    poll_fn(move |cx| {
        for (index, device) in devices.iter_mut().enumerate() {
            if device.ahead.is_some() {
                continue;
            }
            match device.stream.poll_event(cx) {
                Poll::Ready(Ok(event)) => device.ahead = Some(event),
                Poll::Ready(Err(err)) => return Poll::Ready((index, Err(err))),
                Poll::Pending => {}
            }
        }
        // Once the task's budget is spent, a stream says Pending without
        // looking at its events, and an earlier event there would lose its
        // place. Yield and look again with a fresh budget.
        if !coop::has_budget_remaining() {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        let earliest = devices
            .iter()
            .enumerate()
            .filter_map(|(index, device)| {
                let event = device.ahead?;
                Some((!events::is_loss(&event), event.timestamp(), index))
            })
            .min();
        match earliest
            .and_then(|(_, _, index)| Some((index, devices.get_mut(index)?.ahead.take()?)))
        {
            Some((index, event)) => Poll::Ready((index, Ok(event))),
            None => Poll::Pending,
        }
    })
}

/// The most events a keyboard may still have queued when it is read again
/// after lost events; far more than the kernel queues for one reader.
const DRAIN_LIMIT: usize = 65_536;

/// The keys a keyboard holds, read after lost events. Every event still
/// queued for it, in the stream's buffer or the kernel's, is older than
/// the read and already reflected in it, so it is dropped first: replayed
/// on top of the read, an older release would undo a key still held.
/// Reading the key state drops the kernel's queued key events itself, but
/// not those the stream has already read.
///
/// The drain polls with a waker that does nothing; the reader's next wait
/// polls this stream again with its own, before it sleeps.
fn read_after_loss(stream: &mut EventStream) -> io::Result<AttributeSet<KeyCode>> {
    let mut cx = Context::from_waker(Waker::noop());
    let mut drained = 0_usize;
    while let Poll::Ready(event) = stream.poll_event(&mut cx) {
        event?;
        drained = drained.saturating_add(1);
        if drained > DRAIN_LIMIT {
            return Err(io::Error::other(
                "a keyboard sends events faster than they can be read",
            ));
        }
    }
    stream.device().get_key_state()
}

/// A device being read, and the event read from it but not handled yet.
struct Reading {
    id: DeviceId,
    name: OsString,
    role: Role,
    stream: Box<EventStream>,
    ahead: Option<InputEvent>,
    /// Events were lost and the current frame is not over.
    dropped: bool,
}

struct Reader<S> {
    paths: Paths,
    bindings: Bindings,
    own: Vec<PathBuf>,
    feed: RecordingFeed,
    send: S,
    registry: Registry,
    matcher: Matcher,
    devices: Vec<Reading>,
    next_id: u64,
    reported: Option<HotkeyStatus>,
}

impl<S: FnMut(HotkeyEvent)> Reader<S> {
    fn new(paths: Paths, setup: Setup, send: S) -> Self {
        let Setup {
            bindings,
            own,
            feed,
        } = setup;
        Reader {
            paths,
            bindings,
            own,
            feed,
            send,
            registry: Registry::default(),
            matcher: Matcher::new(bindings),
            devices: Vec::new(),
            next_id: 0,
            reported: None,
        }
    }

    /// Opens every node not open yet.
    fn scan(&mut self) -> io::Result<()> {
        for entry in std::fs::read_dir(&self.paths.dev)? {
            self.try_open(&entry?.file_name());
        }
        Ok(())
    }

    fn changed(&mut self, changes: Vec<Change>) -> io::Result<()> {
        for change in changes {
            match change {
                Change::Created(name) | Change::Attrib(name) => self.try_open(&name),
                Change::Deleted(name) => {
                    if let Some(id) = self.registry.deleted(&name) {
                        self.unplugged(id);
                    }
                }
                Change::Overflow => {
                    self.registry.forget_unread();
                    self.scan()?;
                }
            }
        }
        Ok(())
    }

    fn try_open(&mut self, name: &OsStr) {
        if !self.registry.wants_open(name) {
            return;
        }
        let path = self.paths.dev.join(name);
        let wanted = Wanted {
            bindings: &self.bindings,
            recording: self.feed.recording(),
        };
        let opened = match node::open(&path, &self.paths, &self.own, wanted) {
            Node::Device(stream, role) => self.add(name, stream, role),
            Node::Ours => Opened::Ours,
            Node::NotWanted => Opened::NotWanted,
            Node::Gone => Opened::Gone,
            Node::Denied => Opened::Denied,
            Node::Failed(why) => Opened::Failed(why),
        };
        let read_now = match opened {
            Opened::Reading(id, role) if role.record && self.feed.recording() => Some(id),
            _ => None,
        };
        self.registry.record(name, opened);
        // While recording, the recorder learns at once what a device just
        // opened holds.
        if let Some(id) = read_now {
            self.read_state(id);
        }
    }

    fn add(&mut self, name: &OsStr, stream: Box<EventStream>, role: Role) -> Opened {
        let id = DeviceId(self.next_id);
        let Some(next) = self.next_id.checked_add(1) else {
            return Opened::Failed("too many devices".to_owned());
        };
        self.next_id = next;
        self.devices.push(Reading {
            id,
            name: name.to_owned(),
            role,
            stream,
            ahead: None,
            dropped: false,
        });
        Opened::Reading(id, role)
    }

    fn event(&mut self, index: usize, event: io::Result<InputEvent>) {
        let Some(device) = self.devices.get_mut(index) else {
            return;
        };
        let (id, record) = (device.id, device.role.record);
        let event = match event {
            Ok(event) => event,
            Err(err) => return self.read_failed(id, &err),
        };
        let time = events::time(&event);
        let feed = if record { Some(&self.feed) } else { None };
        let record = |input: &mut dyn FnMut(&mut Recorder)| {
            if let Some(feed) = feed {
                feed.with(|recorder| input(recorder));
            }
        };
        match events::step(&mut device.dropped, &event) {
            Step::Key(key, change) => {
                let reaction = self.matcher.key(id, key);
                if let Some((code, pressed)) = change {
                    record(&mut |recorder| {
                        let input = KeyInput {
                            fired: reaction.fired,
                            ..KeyInput::new(code, pressed)
                        };
                        recorder.key(id, input, time);
                    });
                }
                self.send_reaction(reaction);
            }
            Step::Motion { dx, dy } => record(&mut |recorder| recorder.motion(time, dx, dy)),
            Step::Wheel(notches) => record(&mut |recorder| recorder.wheel(time, notches)),
            Step::Lost => {
                let reaction = self.matcher.events_lost(id);
                self.send_reaction(reaction);
            }
            Step::Resync => self.read_state(id),
            Step::Nothing => {}
        }
    }

    /// Reads what a device holds now, after lost events or as a recording
    /// starts, and tells the matcher and the recorder. Every event read
    /// from it before is older than the read, and dropped.
    fn read_state(&mut self, id: DeviceId) {
        let Some(device) = self.devices.iter_mut().find(|device| device.id == id) else {
            return;
        };
        device.ahead = None;
        device.dropped = false;
        let record = device.role.record;
        match read_after_loss(&mut device.stream) {
            Ok(keys) => {
                let held = |code| keys.contains(KeyCode(code));
                let now = monotonic_now();
                self.matcher.resync(id, held, now);
                if record {
                    self.feed.with(|recorder| recorder.resync(id, held, now));
                }
            }
            Err(err) => self.read_failed(id, &err),
        }
    }

    /// Reading a device failed. An unplugged device's node is looked at
    /// again, since it may already be a new device's; any other failure
    /// waits for the node's attributes to change.
    fn read_failed(&mut self, id: DeviceId, err: &io::Error) {
        let Some(name) = self.remove(id) else {
            return;
        };
        if node::is_gone(err) {
            self.registry.read_failed(&name, None);
            self.try_open(&name);
        } else {
            self.registry.read_failed(&name, Some(err.to_string()));
        }
    }

    /// Stops reading an unplugged device.
    fn unplugged(&mut self, id: DeviceId) {
        self.remove(id);
    }

    /// Stops reading a device, and forgets the keys it held; returns its
    /// node's name.
    fn remove(&mut self, id: DeviceId) -> Option<OsString> {
        let index = self.devices.iter().position(|device| device.id == id)?;
        let name = self.devices.swap_remove(index).name;
        let now = monotonic_now();
        let reaction = self.matcher.device_lost(id, now);
        self.send_reaction(reaction);
        self.feed.with(|recorder| recorder.device_lost(id, now));
        Some(name)
    }

    fn send_reaction(&mut self, reaction: Reaction) {
        reaction.events().for_each(&mut self.send);
    }

    /// Sends the status when it changed.
    fn report(&mut self) {
        let status = self.registry.status();
        if self.reported.as_ref() != Some(&status) {
            (self.send)(HotkeyEvent::Status(status.clone()));
            self.reported = Some(status);
        }
    }

    /// Ends after watching failed: the status says why hotkeys stopped. No
    /// release is sent for keys still held; the status ends any wait for
    /// one.
    fn stop(mut self, err: &io::Error) {
        let why = Unavailable::WatchFailed(err.to_string());
        (self.send)(HotkeyEvent::Status(HotkeyStatus::Unavailable(why)));
    }
}

#[cfg(test)]
mod tests;
