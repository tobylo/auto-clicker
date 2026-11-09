//! Global hotkeys on the kernel backend: every keyboard in `/dev/input`,
//! read without grabbing it.
//!
//! [`watch_hotkeys`] opens each node with a bound key or a modifier key,
//! except this program's own virtual devices, and watches the directory for
//! keyboards coming and going. Key events go to one [`Matcher`] for all
//! keyboards; what it says goes to the controller. Keys reach the game too:
//! nothing is grabbed.

mod events;
mod node;
mod registry;
mod watch;

use std::ffi::{OsStr, OsString};
use std::future::{Future, poll_fn};
use std::io;
use std::path::PathBuf;
use std::pin::pin;
use std::task::Poll;

use auto_clicker_core::hotkeys::{
    Bindings, DeviceId, HotkeyEvent, HotkeyStatus, Matcher, Reaction, Unavailable,
};
use evdev::raw_stream::EventStream;
use evdev::{InputEvent, KeyCode};

use crate::DeviceIdentity;
use events::Step;
use node::{Node, Paths};
use registry::{Cause, Opened, Registry};
use watch::{Change, Watch};

/// Reads every keyboard for hotkeys until `shutdown` resolves, and hands
/// what happens to `send`: fired bindings, the release after each, and
/// whether hotkeys are available.
///
/// `own` must hold every virtual device this program has open, and be
/// complete before this starts: a device created later would be read like
/// any other. A backend that is recreated needs this restarted with the new
/// identities.
///
/// Runs inside a tokio runtime with I/O enabled. `send` must not block.
/// When watching `/dev/input` fails, it reports
/// [`Unavailable::WatchFailed`] and returns.
pub async fn watch_hotkeys(
    bindings: Bindings,
    own: &[DeviceIdentity],
    send: impl FnMut(HotkeyEvent),
    shutdown: impl Future<Output = ()>,
) {
    let own = own.iter().map(|device| device.syspath.clone()).collect();
    run(Reader::new(Paths::system(), bindings, own, send), shutdown).await;
}

/// What woke the reader.
enum Wake {
    ShutDown,
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
    if let Err(err) = reader.scan() {
        return reader.stop(&err);
    }
    loop {
        reader.report();
        let wake = tokio::select! {
            biased;
            () = &mut shutdown => Wake::ShutDown,
            changes = watch.changes() => Wake::Changes(changes),
            (index, event) = next_event(&mut reader.devices) => Wake::Event(index, event),
        };
        let handled = match wake {
            Wake::ShutDown => return,
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
/// on another keep their order however late the reader looks. An error is
/// returned at once. Never resolves while no event waits.
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
        let earliest = devices
            .iter()
            .enumerate()
            .filter_map(|(index, device)| device.ahead.map(|event| (event.timestamp(), index)))
            .min();
        match earliest.and_then(|(_, index)| Some((index, devices.get_mut(index)?.ahead.take()?))) {
            Some((index, event)) => Poll::Ready((index, Ok(event))),
            None => Poll::Pending,
        }
    })
}

/// A keyboard being read, and the event read from it but not handled yet.
struct Reading {
    id: DeviceId,
    name: OsString,
    stream: Box<EventStream>,
    ahead: Option<InputEvent>,
    /// Events were lost and the current frame is not over.
    dropped: bool,
}

struct Reader<S> {
    paths: Paths,
    bindings: Bindings,
    own: Vec<PathBuf>,
    send: S,
    registry: Registry,
    matcher: Matcher,
    devices: Vec<Reading>,
    next_id: u64,
    reported: Option<HotkeyStatus>,
}

impl<S: FnMut(HotkeyEvent)> Reader<S> {
    fn new(paths: Paths, bindings: Bindings, own: Vec<PathBuf>, send: S) -> Self {
        Reader {
            paths,
            bindings,
            own,
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
            self.try_open(&entry?.file_name(), Cause::Scan);
        }
        Ok(())
    }

    fn changed(&mut self, changes: Vec<Change>) -> io::Result<()> {
        for change in changes {
            match change {
                Change::Created(name) => self.try_open(&name, Cause::Created),
                Change::Attrib(name) => self.try_open(&name, Cause::Attrib),
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

    fn try_open(&mut self, name: &OsStr, cause: Cause) {
        if !self.registry.wants_open(name) {
            return;
        }
        let path = self.paths.dev.join(name);
        let opened = match node::open(&path, &self.paths, &self.own, &self.bindings) {
            Node::Keyboard(stream) => self.add(name, stream),
            Node::Ours => Opened::Ours,
            Node::NotKeyboard => Opened::NotKeyboard,
            Node::Gone => Opened::Gone,
            Node::Denied { udev_done } => Opened::Denied { udev_done },
            Node::Failed(why) => Opened::Failed(why),
        };
        self.registry.record(name, opened, cause);
    }

    fn add(&mut self, name: &OsStr, stream: Box<EventStream>) -> Opened {
        let id = DeviceId(self.next_id);
        let Some(next) = self.next_id.checked_add(1) else {
            return Opened::Failed("too many keyboards".to_owned());
        };
        self.next_id = next;
        self.devices.push(Reading {
            id,
            name: name.to_owned(),
            stream,
            ahead: None,
            dropped: false,
        });
        Opened::Keyboard(id)
    }

    fn event(&mut self, index: usize, event: io::Result<InputEvent>) {
        let Some(device) = self.devices.get_mut(index) else {
            return;
        };
        let id = device.id;
        let event = match event {
            Ok(event) => event,
            Err(err) => return self.read_failed(id, &err),
        };
        match events::step(&mut device.dropped, &event) {
            Step::Key(key) => {
                let reaction = self.matcher.key(id, key);
                self.send_reaction(reaction);
            }
            Step::Resync => match device.stream.device().get_key_state() {
                Ok(keys) => {
                    let reaction = self.matcher.resync(id, |code| keys.contains(KeyCode(code)));
                    self.send_reaction(reaction);
                }
                Err(err) => self.read_failed(id, &err),
            },
            Step::Nothing => {}
        }
    }

    /// Reading a device failed. An unplugged device lets go of its keys and
    /// its node is looked at again, since it may already be a new device's.
    /// Any other failure forgets its keys without a release, and the node
    /// waits for its attributes to change.
    fn read_failed(&mut self, id: DeviceId, err: &io::Error) {
        let Some(name) = self.remove(id) else {
            return;
        };
        if node::is_gone(err) {
            let reaction = self.matcher.device_gone(id);
            self.send_reaction(reaction);
            self.registry.read_failed(&name, None);
            self.try_open(&name, Cause::Scan);
        } else {
            let reaction = self.matcher.device_failed(id);
            self.send_reaction(reaction);
            self.registry.read_failed(&name, Some(err.to_string()));
        }
    }

    /// Stops reading an unplugged device and lets go of the keys it held.
    fn unplugged(&mut self, id: DeviceId) {
        self.remove(id);
        let reaction = self.matcher.device_gone(id);
        self.send_reaction(reaction);
    }

    /// Stops reading a device; returns its node's name.
    fn remove(&mut self, id: DeviceId) -> Option<OsString> {
        let index = self.devices.iter().position(|device| device.id == id)?;
        Some(self.devices.swap_remove(index).name)
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
