//! libei output over an EIS socket: the shape a portal backend would take.
//!
//! The `InputBackend` methods are synchronous, and so are reis's requests:
//! each one is buffered and `flush` writes it to the non-blocking socket. What
//! needs the runtime is the event stream: the compositor pings the client,
//! announces devices, and pauses or resumes them, and nobody acts on that
//! unless something polls the stream. Here one local task drains it and keeps
//! [`Devices`] current; the emit calls only read that state.

use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::time::Duration;

use enumflags2::BitFlags;
use futures_util::StreamExt;
use reis::ei;
use reis::event::{Connection, Device, DeviceCapability, EiEvent};
use reis::tokio::EiConvertEventStream;
use tokio::sync::Notify;

use crate::log::{line, now_us};

/// What the drain task knows about the connection and its devices.
#[derive(Default)]
struct Devices {
    list: Vec<Tracked>,
    disconnected: Option<String>,
}

struct Tracked {
    device: Device,
    resumed: bool,
    emulating: bool,
}

/// A libei sender connection with its devices.
pub struct Output {
    connection: Connection,
    state: Rc<RefCell<Devices>>,
    changed: Rc<Notify>,
    sequence: u32,
}

const ALL: [DeviceCapability; 5] = [
    DeviceCapability::Pointer,
    DeviceCapability::PointerAbsolute,
    DeviceCapability::Keyboard,
    DeviceCapability::Scroll,
    DeviceCapability::Button,
];

impl Output {
    /// Shakes hands as a sender and starts the task that drains events.
    /// Must run inside a `LocalSet`.
    pub async fn connect(fd: OwnedFd) -> Result<Output, String> {
        let context = ei::Context::new(UnixStream::from(fd)).map_err(|e| format!("ei: {e}"))?;
        let (connection, events) = context
            .handshake_tokio("auto-clicker-x02", ei::handshake::ContextType::Sender)
            .await
            .map_err(|e| format!("ei handshake: {e}"))?;
        line("ei handshake done");
        let state = Rc::new(RefCell::new(Devices::default()));
        let changed = Rc::new(Notify::new());
        tokio::task::spawn_local(drain(
            events,
            connection.clone(),
            state.clone(),
            changed.clone(),
        ));
        Ok(Output {
            connection,
            state,
            changed,
            sequence: 0,
        })
    }

    /// Waits until a resumed device offers each of `capabilities`.
    pub async fn wait_for(&self, capabilities: &[DeviceCapability]) -> Result<(), String> {
        let wait = async {
            while !self.has_all(capabilities) {
                if let Some(reason) = &self.state.borrow().disconnected {
                    return Err(format!("disconnected: {reason}"));
                }
                self.changed.notified().await;
            }
            Ok(())
        };
        tokio::time::timeout(Duration::from_secs(10), wait)
            .await
            .map_err(|_| format!("no resumed device with {capabilities:?} within 10 s"))?
    }

    fn has_all(&self, capabilities: &[DeviceCapability]) -> bool {
        let state = self.state.borrow();
        capabilities.iter().all(|cap| {
            state
                .list
                .iter()
                .any(|t| t.resumed && t.device.has_capability(*cap))
        })
    }

    /// Prints each device with its capabilities and regions.
    pub fn describe(&self) {
        for tracked in &self.state.borrow().list {
            let device = &tracked.device;
            let caps: Vec<_> = ALL
                .iter()
                .filter(|cap| device.has_capability(**cap))
                .collect();
            line(&format!(
                "device {:?} {:?}: {caps:?}, regions {:?}",
                device.name(),
                device.device_type(),
                device.regions()
            ));
        }
    }

    /// The regions of the absolute pointer, for mapping a normalised point.
    pub fn absolute_regions(&self) -> Vec<(u32, u32, u32, u32)> {
        let state = self.state.borrow();
        state
            .list
            .iter()
            .filter(|t| t.device.has_capability(DeviceCapability::PointerAbsolute))
            .flat_map(|t| {
                t.device
                    .regions()
                    .iter()
                    .map(|r| (r.x, r.y, r.width, r.height))
            })
            .collect()
    }

    pub fn button(&mut self, code: u32, down: bool) -> Result<(), String> {
        let state = if down {
            ei::button::ButtonState::Press
        } else {
            ei::button::ButtonState::Released
        };
        self.emit(DeviceCapability::Button, |d| {
            interface::<ei::Button>(d).map(|b| b.button(code, state))
        })
    }

    pub fn key(&mut self, code: u32, down: bool) -> Result<(), String> {
        let state = if down {
            ei::keyboard::KeyState::Press
        } else {
            ei::keyboard::KeyState::Released
        };
        self.emit(DeviceCapability::Keyboard, |d| {
            interface::<ei::Keyboard>(d).map(|k| k.key(code, state))
        })
    }

    pub fn move_relative(&mut self, dx: f32, dy: f32) -> Result<(), String> {
        self.emit(DeviceCapability::Pointer, |d| {
            interface::<ei::Pointer>(d).map(|p| p.motion_relative(dx, dy))
        })
    }

    pub fn move_absolute(&mut self, x: f32, y: f32) -> Result<(), String> {
        self.emit(DeviceCapability::PointerAbsolute, |d| {
            interface::<ei::PointerAbsolute>(d).map(|p| p.motion_absolute(x, y))
        })
    }

    /// One wheel notch is 120 in libei's discrete units; positive is down.
    pub fn scroll_notches(&mut self, notches: i32) -> Result<(), String> {
        let value = notches.saturating_mul(120);
        self.emit(DeviceCapability::Scroll, |d| {
            interface::<ei::Scroll>(d).map(|s| s.scroll_discrete(0, value))
        })
    }

    /// Ends emulation on every device, as a client does when it stops.
    pub fn stop_emulating(&mut self) -> Result<(), String> {
        let serial = self.connection.serial();
        for tracked in self.state.borrow_mut().list.iter_mut() {
            if tracked.emulating {
                tracked.device.device().stop_emulating(serial);
                tracked.emulating = false;
            }
        }
        self.flush()
    }

    /// Closes the EIS socket only, keeping the portal session.
    pub fn disconnect(&self) -> Result<(), String> {
        self.connection.connection().disconnect();
        self.flush()
    }

    /// Why the compositor ended the connection, once it has.
    pub fn disconnected(&self) -> Option<String> {
        self.state.borrow().disconnected.clone()
    }

    /// Writes one request on the first resumed device with `capability`,
    /// starting emulation first if needed, then ends the frame and flushes.
    fn emit(
        &mut self,
        capability: DeviceCapability,
        request: impl FnOnce(&Device) -> Option<()>,
    ) -> Result<(), String> {
        let serial = self.connection.serial();
        let device = self.start_device(capability, serial)?;
        request(&device).ok_or_else(|| format!("{capability:?}: interface missing"))?;
        device.device().frame(serial, now_us());
        self.flush()
    }

    fn start_device(
        &mut self,
        capability: DeviceCapability,
        serial: u32,
    ) -> Result<Device, String> {
        let mut state = self.state.borrow_mut();
        let tracked = state
            .list
            .iter_mut()
            .find(|t| t.resumed && t.device.has_capability(capability))
            .ok_or_else(|| format!("no resumed device with {capability:?}"))?;
        if !tracked.emulating {
            tracked
                .device
                .device()
                .start_emulating(serial, self.sequence);
            self.sequence = self.sequence.wrapping_add(1);
            tracked.emulating = true;
        }
        Ok(tracked.device.clone())
    }

    fn flush(&self) -> Result<(), String> {
        self.connection
            .flush()
            .map_err(|err| format!("flush: {err}"))
    }
}

fn interface<T: ei::Interface>(device: &Device) -> Option<T> {
    device.interface::<T>()
}

/// Reads events until the connection ends, keeping `state` current. Replies
/// the converter queues (pings) and seat binds are only buffered, so the task
/// flushes after every event.
async fn drain(
    mut events: EiConvertEventStream,
    connection: Connection,
    state: Rc<RefCell<Devices>>,
    changed: Rc<Notify>,
) {
    while let Some(event) = events.next().await {
        match event {
            Ok(event) => {
                apply(&mut state.borrow_mut(), event);
                if let Err(err) = connection.flush() {
                    line(&format!("ei: flush after event failed: {err}"));
                }
            }
            Err(err) => {
                state.borrow_mut().disconnected = Some(format!("stream error: {err}"));
                break;
            }
        }
        changed.notify_waiters();
    }
    let mut state = state.borrow_mut();
    state
        .disconnected
        .get_or_insert_with(|| "socket closed".to_owned());
    line(&format!("ei: connection ended ({:?})", state.disconnected));
    drop(state);
    changed.notify_waiters();
}

fn apply(state: &mut Devices, event: EiEvent) {
    match event {
        EiEvent::SeatAdded(added) => bind_all(&added.seat),
        EiEvent::DeviceAdded(added) => track(state, added.device),
        EiEvent::DeviceResumed(resumed) => set_resumed(state, &resumed.device, true),
        EiEvent::DevicePaused(paused) => set_resumed(state, &paused.device, false),
        EiEvent::DeviceRemoved(removed) => untrack(state, &removed.device),
        EiEvent::Disconnected(gone) => {
            line(&format!(
                "ei: disconnected {:?} {:?}",
                gone.reason, gone.explanation
            ));
            state.disconnected = Some(format!("{:?} {:?}", gone.reason, gone.explanation));
        }
        other => line(&format!("ei: {other:?}")),
    }
}

fn bind_all(seat: &reis::event::Seat) {
    line(&format!("ei: seat {:?}", seat.name()));
    seat.bind_capabilities(ALL.iter().copied().collect::<BitFlags<_>>());
}

fn track(state: &mut Devices, device: Device) {
    line(&format!("ei: device added {:?}", device.name()));
    state.list.push(Tracked {
        device,
        resumed: false,
        emulating: false,
    });
}

fn set_resumed(state: &mut Devices, device: &Device, resumed: bool) {
    line(&format!("ei: device {:?} resumed={resumed}", device.name()));
    if let Some(tracked) = state.list.iter_mut().find(|t| &t.device == device) {
        tracked.resumed = resumed;
        // A paused device stops emulating; it must start again on resume.
        tracked.emulating = false;
    }
}

fn untrack(state: &mut Devices, device: &Device) {
    line(&format!("ei: device removed {:?}", device.name()));
    state.list.retain(|t| &t.device != device);
}
