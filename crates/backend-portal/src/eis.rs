//! libei output over the EIS socket the portal hands out.
//!
//! Writes are synchronous: reis buffers each request and `flush` writes it
//! to the non-blocking socket. What needs the runtime is the event stream:
//! the compositor pings the client, announces, pauses and removes devices,
//! and ends the connection, and nothing answers unless the stream is polled.
//! One future drains it, keeps the device table current and flushes after
//! every event (without that flush the seat bind and the ping replies stay
//! in the buffer, and no device ever appears; X02).
//!
//! The drain belongs to the [`Eis`] and ends with it; it is not a task of
//! its own (reis's stream is not `Send`, and a spawned task could outlive
//! the session). Whatever waits on the session drives it: the open while it
//! waits for the devices, then the future [`Eis::lost`] returns, which the
//! controller polls for as long as a run has the devices.

use std::cell::RefCell;
use std::future::{Future, poll_fn};
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;
use std::time::Duration;

use auto_clicker_core::input::InputError;
use futures_util::StreamExt;
use reis::ei;
use reis::event::{Connection, Device, DeviceCapability, EiEvent};
use reis::tokio::EiConvertEventStream;
use tokio::sync::watch;
use tokio::time::timeout;

use crate::layout::{Layout, Region};

/// How long the handshake may take.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the compositor may take to announce and resume the devices.
const DEVICES_TIMEOUT: Duration = Duration::from_secs(5);

/// The name the compositor shows for this client.
const CLIENT_NAME: &str = "auto-clicker";

/// Every capability the program binds: what the clicker and playback use.
const BOUND: [DeviceCapability; 5] = [
    DeviceCapability::Pointer,
    DeviceCapability::PointerAbsolute,
    DeviceCapability::Button,
    DeviceCapability::Scroll,
    DeviceCapability::Keyboard,
];

/// What the drain task knows about the devices.
#[derive(Default)]
struct Table {
    devices: Vec<Tracked>,
}

struct Tracked {
    device: Device,
    resumed: bool,
    emulating: bool,
}

/// Why the connection is lost: `None` while it works.
type Lost = Option<String>;

/// A libei sender connection with its devices.
pub(crate) struct Eis {
    connection: Connection,
    table: Arc<Mutex<Table>>,
    lost: watch::Receiver<Lost>,
    drain: Drain,
    /// The next emulation sequence number.
    sequence: u32,
}

impl Eis {
    /// Shakes hands over `socket` as a sender, starts draining its events,
    /// and waits until resumed devices offer every capability in `needed`.
    /// Each wait has its own deadline; on any failure, everything started
    /// so far ends with the returned error.
    pub(crate) async fn connect(
        socket: OwnedFd,
        needed: &[DeviceCapability],
    ) -> Result<Eis, InputError> {
        let context = ei::Context::new(UnixStream::from(socket))
            .map_err(|err| InputError::Portal(format!("EIS socket: {err}")))?;
        let (connection, events) = timeout(
            HANDSHAKE_TIMEOUT,
            context.handshake_tokio(CLIENT_NAME, ei::handshake::ContextType::Sender),
        )
        .await
        .map_err(|_| InputError::Portal("EIS handshake: no answer within 5 s".to_owned()))?
        .map_err(|err| InputError::Portal(format!("EIS handshake: {err}")))?;
        let table = Arc::new(Mutex::new(Table::default()));
        let (report, lost) = watch::channel(None);
        let drain = Drain::new(drain(events, connection.clone(), table.clone(), report));
        let eis = Eis {
            connection,
            table,
            lost,
            drain,
            sequence: 0,
        };
        eis.wait_for(needed).await?;
        Ok(eis)
    }

    /// Waits until resumed devices offer every capability in `needed`.
    async fn wait_for(&self, needed: &[DeviceCapability]) -> Result<(), InputError> {
        let mut lost = self.lost.clone();
        let ready = async {
            loop {
                if let Some(why) = lost.borrow_and_update().clone() {
                    return Err(InputError::SessionLost(why));
                }
                if needed.iter().all(|&capability| self.offers(capability)) {
                    return Ok(());
                }
                // Every table change is followed by a send on the watch.
                if lost.changed().await.is_err() {
                    return Err(InputError::SessionLost("the connection closed".to_owned()));
                }
            }
        };
        let driven = async {
            tokio::select! {
                result = ready => result,
                // The drain ended, so the watch says why.
                () = self.drain.drive() => Err(InputError::SessionLost(
                    self.lost.borrow().clone().unwrap_or_default(),
                )),
            }
        };
        timeout(DEVICES_TIMEOUT, driven).await.unwrap_or_else(|_| {
            Err(InputError::Portal(format!(
                "the desktop offered no input devices within {} s",
                DEVICES_TIMEOUT.as_secs()
            )))
        })
    }

    fn offers(&self, capability: DeviceCapability) -> bool {
        lock(&self.table)
            .devices
            .iter()
            .any(|tracked| tracked.resumed && tracked.device.has_capability(capability))
    }

    /// Resolves once the connection is lost, with why. It drives the
    /// event stream meanwhile, so it must be polled while the devices are
    /// in use.
    pub(crate) fn lost(&self) -> impl Future<Output = InputError> + 'static {
        let drain = self.drain.clone();
        let lost = self.lost.clone();
        async move {
            drain.drive().await;
            let why = lost.borrow().clone();
            InputError::SessionLost(why.unwrap_or_else(|| "the connection closed".to_owned()))
        }
    }

    /// The layout of the absolute pointer's outputs.
    pub(crate) fn layout(&self) -> Result<Layout, InputError> {
        let table = lock(&self.table);
        let device = table
            .devices
            .iter()
            .find(|t| t.resumed && t.device.has_capability(DeviceCapability::PointerAbsolute))
            .ok_or_else(|| no_device("move the pointer to a point"))?;
        Layout::new(device.device.regions().iter().map(|r| Region {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }))
        .ok_or_else(|| {
            InputError::Portal("the desktop reported no screens for the pointer".to_owned())
        })
    }

    /// Writes one request on a resumed device with `capability`, starting
    /// emulation on it first if needed, then ends the frame and flushes.
    pub(crate) fn emit<T: ei::Interface>(
        &mut self,
        capability: DeviceCapability,
        what: &str,
        request: impl FnOnce(&T),
    ) -> Result<(), InputError> {
        if let Some(why) = self.lost.borrow().clone() {
            return Err(InputError::SessionLost(why));
        }
        let serial = self.connection.serial();
        let device = self.start_device(capability, serial, what)?;
        let interface = device.interface::<T>().ok_or_else(|| no_device(what))?;
        request(&interface);
        device.device().frame(serial, now_us());
        self.flush()
    }

    fn start_device(
        &mut self,
        capability: DeviceCapability,
        serial: u32,
        what: &str,
    ) -> Result<Device, InputError> {
        let mut table = lock(&self.table);
        let tracked = table
            .devices
            .iter_mut()
            .find(|t| t.resumed && t.device.has_capability(capability))
            .ok_or_else(|| no_device(what))?;
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

    fn flush(&self) -> Result<(), InputError> {
        self.connection
            .flush()
            .map_err(|errno| InputError::Write(io::Error::from(errno)))
    }
}

impl Drop for Eis {
    /// Ends the connection; the compositor then releases whatever its
    /// devices still hold (X02). The drain ends with the last of `self`
    /// and the loss watches.
    fn drop(&mut self) {
        self.connection.connection().disconnect();
        let _ = self.connection.flush();
    }
}

/// The event drain, shared by whatever waits on the session; only one of
/// them polls it at a time (the open, then the run's loss watch).
#[derive(Clone)]
struct Drain(Rc<RefCell<Option<DrainFuture>>>);

/// The drain itself; `None` in the slot once it has ended.
type DrainFuture = Pin<Box<dyn Future<Output = ()>>>;

impl Drain {
    fn new(drain: impl Future<Output = ()> + 'static) -> Drain {
        Drain(Rc::new(RefCell::new(Some(Box::pin(drain)))))
    }

    /// Drives the drain until it ends: when the connection is lost.
    async fn drive(&self) {
        poll_fn(|cx| {
            let mut slot = self.0.borrow_mut();
            let Some(drain) = slot.as_mut() else {
                return Poll::Ready(());
            };
            let polled = drain.as_mut().poll(cx);
            if polled.is_ready() {
                *slot = None;
            }
            polled
        })
        .await;
    }
}

fn no_device(what: &str) -> InputError {
    InputError::Portal(format!(
        "the desktop offers no device that can {what} right now"
    ))
}

/// The table, even if a panic poisoned its lock: it holds no invariant a
/// half-done change could break.
fn lock(table: &Mutex<Table>) -> MutexGuard<'_, Table> {
    table.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The monotonic clock in microseconds, as libei stamps frames.
fn now_us() -> u64 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let secs = u64::try_from(now.tv_sec).unwrap_or(0);
    let micros = u64::try_from(now.tv_nsec / 1000).unwrap_or(0);
    secs.saturating_mul(1_000_000).saturating_add(micros)
}

/// Reads events until the connection ends, keeping `table` current, and
/// reports on `lost` why it ended. Replies the converter queues (pings) and
/// seat binds are only buffered, so it flushes after every event; a failed
/// flush means the socket is gone.
async fn drain(
    mut events: EiConvertEventStream,
    connection: Connection,
    table: Arc<Mutex<Table>>,
    lost: watch::Sender<Lost>,
) {
    let why = loop {
        let Some(event) = events.next().await else {
            break "the connection closed".to_owned();
        };
        let ended = match event {
            Ok(event) => apply(&mut lock(&table), event),
            Err(err) => Some(format!("the connection failed: {err}")),
        };
        if let Some(why) = ended {
            break why;
        }
        if let Err(err) = connection.flush() {
            break format!("the connection failed: {err}");
        }
        // Wakes anyone waiting for devices; the value is still `None`.
        lost.send_modify(|_| {});
    };
    lost.send_replace(Some(why));
}

/// Applies one event; `Some(why)` when it ends the connection's use.
fn apply(table: &mut Table, event: EiEvent) -> Option<String> {
    match event {
        EiEvent::SeatAdded(added) => {
            added
                .seat
                .bind_capabilities(BOUND.iter().copied().collect());
        }
        EiEvent::DeviceAdded(added) => table.devices.push(Tracked {
            device: added.device,
            resumed: false,
            emulating: false,
        }),
        EiEvent::DeviceResumed(resumed) => {
            if let Some(tracked) = find(table, &resumed.device) {
                tracked.resumed = true;
            }
        }
        // A pause ends the run (it may come with a screen lock); what the
        // compositor does with held input then is its own, so the run
        // stops rather than waits.
        EiEvent::DevicePaused(paused) => {
            if find(table, &paused.device).is_some_and(|tracked| tracked.resumed) {
                return Some("the desktop paused its input devices".to_owned());
            }
        }
        // Removing a device the run uses ends it: a hold run writes nothing
        // more, and the compositor released what the device held. Any other
        // removal is not a loss: the compositor may add the device again,
        // as when a monitor is plugged in.
        EiEvent::DeviceRemoved(removed) => {
            if find(table, &removed.device).is_some_and(|tracked| tracked.emulating) {
                return Some("the desktop removed an input device in use".to_owned());
            }
            table
                .devices
                .retain(|tracked| tracked.device != removed.device);
        }
        EiEvent::Disconnected(gone) => {
            return Some(match gone.explanation {
                Some(explanation) => format!("{:?}: {explanation}", gone.reason),
                None => format!("{:?}", gone.reason),
            });
        }
        _ => {}
    }
    None
}

fn find<'a>(table: &'a mut Table, device: &Device) -> Option<&'a mut Tracked> {
    table
        .devices
        .iter_mut()
        .find(|tracked| tracked.device == *device)
}
