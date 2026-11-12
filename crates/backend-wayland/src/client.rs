//! The Wayland connection: what it binds, what it hears, and how it waits.
//!
//! It binds every `wl_output` and its xdg-output, and feeds what they say to
//! [`Gather`]. It waits on the connection's descriptor through the runtime,
//! so it wakes only on the compositor's events (brief W7). Any error ends it:
//! a closed or failing socket is a lost connection, anything else the
//! compositor or the library refuses is a protocol error (brief W5).

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::os::unix::net::UnixStream;

use auto_clicker_core::recording::Layout;
use tokio::io::Interest;
use wayland_client::backend::{ReadEventsGuard, WaylandError};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_output::{self, Transform, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};

use crate::Missing;
use crate::gather::{Gather, Heard};
use crate::socket::{self, Waiting};

/// The newest versions the client knows: `wl_output` 4 (name and
/// description, which it ignores), xdg-output 3 (`done` on `wl_output`).
const OUTPUT_VERSION: u32 = 4;
const XDG_OUTPUT_VERSION: u32 = 3;
const OUTPUT: &str = "wl_output";
const XDG_OUTPUT_MANAGER: &str = "zxdg_output_manager_v1";

/// Which round trip a `wl_display.sync` callback ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// The first, after which every global at start is known.
    Globals,
    /// One asked for by [`Gather`].
    Layout,
}

/// An output and its xdg-output.
struct Bound {
    output: WlOutput,
    xdg: ZxdgOutputV1,
}

/// A global the compositor offers.
#[derive(Clone, Copy, Debug)]
struct Global {
    name: u32,
    version: u32,
}

/// What the event handlers keep.
#[derive(Default)]
pub(crate) struct Client {
    pub(crate) gather: Gather,
    /// The outputs offered before the manager was bound.
    offered: Vec<Global>,
    /// The xdg-output manager, once offered.
    manager_offered: Option<Global>,
    manager: Option<ZxdgOutputManagerV1>,
    outputs: BTreeMap<u32, Bound>,
    /// The first round trip came back.
    globals_known: bool,
    /// The layout a round trip settled on, until it is taken.
    pub(crate) settled: Option<Option<Layout>>,
}

/// An event as it arrived: for which object, with that object's data, and
/// the queue any object it makes goes on.
struct Arrived<'a, I: Proxy, U> {
    proxy: &'a I,
    event: I::Event,
    data: &'a U,
    queue: &'a QueueHandle<Client>,
}

/// Hands the events of `$interface`'s objects, whose data is `$data`, to
/// `Client::$handler`: the five interfaces share `Dispatch`'s signature.
macro_rules! dispatch_to {
    ($interface:ty, $data:ty, $handler:ident) => {
        impl Dispatch<$interface, $data> for Client {
            fn event(
                client: &mut Self,
                proxy: &$interface,
                event: <$interface as Proxy>::Event,
                data: &$data,
                _: &Connection,
                queue: &QueueHandle<Self>,
            ) {
                client.$handler(Arrived {
                    proxy,
                    event,
                    data,
                    queue,
                });
            }
        }
    };
}

dispatch_to!(WlRegistry, (), registry_event);
dispatch_to!(WlOutput, u32, output_event);
dispatch_to!(ZxdgOutputV1, u32, xdg_output_event);
dispatch_to!(ZxdgOutputManagerV1, (), manager_event);
dispatch_to!(WlCallback, Mark, callback_event);

impl Client {
    /// Binds the outputs offered at start and their xdg-outputs.
    fn bind_offered(
        &mut self,
        registry: &WlRegistry,
        queue: &QueueHandle<Client>,
    ) -> Result<(), Missing> {
        let manager = self.manager_offered.ok_or(Missing::NoXdgOutput)?;
        let version = manager.version.min(XDG_OUTPUT_VERSION);
        self.manager = Some(registry.bind(manager.name, version, queue, ()));
        for output in std::mem::take(&mut self.offered) {
            self.bind_output(registry, queue, output);
        }
        self.gather.start();
        Ok(())
    }

    /// Binds the output `global` and its xdg-output; once the manager is
    /// bound, else keeps it for [`Client::bind_offered`].
    fn bind_output(&mut self, registry: &WlRegistry, queue: &QueueHandle<Client>, global: Global) {
        let Some(manager) = &self.manager else {
            self.offered.push(global);
            return;
        };
        let Global { name, version } = global;
        let output: WlOutput = registry.bind(name, version.min(OUTPUT_VERSION), queue, name);
        let xdg = manager.get_xdg_output(&output, queue, name);
        self.outputs.insert(name, Bound { output, xdg });
        self.gather.added(name);
    }

    /// The global `name` went: an output's objects are released, and an
    /// output or manager offered but not bound yet is never bound.
    fn remove_global(&mut self, name: u32) {
        self.offered.retain(|global| global.name != name);
        if self
            .manager_offered
            .is_some_and(|manager| manager.name == name)
        {
            self.manager_offered = None;
        }
        let Some(Bound { output, xdg }) = self.outputs.remove(&name) else {
            return;
        };
        xdg.destroy();
        if output.version() >= 3 {
            output.release();
        }
        self.gather.removed(name);
    }

    fn registry_event(&mut self, arrived: Arrived<'_, WlRegistry, ()>) {
        match arrived.event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                let global = Global { name, version };
                match interface.as_str() {
                    OUTPUT => self.bind_output(arrived.proxy, arrived.queue, global),
                    XDG_OUTPUT_MANAGER => self.manager_offered = Some(global),
                    _ => {}
                }
            }
            wl_registry::Event::GlobalRemove { name } => self.remove_global(name),
            _ => {}
        }
    }

    fn output_event(&mut self, arrived: Arrived<'_, WlOutput, u32>) {
        let heard = match arrived.event {
            wl_output::Event::Geometry { transform, .. } => Heard::Turned(turned(transform)),
            wl_output::Event::Mode {
                flags,
                width,
                height,
                ..
            } if current(flags) => Heard::Mode { width, height },
            wl_output::Event::Scale { factor } => Heard::Scale(factor),
            wl_output::Event::Done => Heard::Done,
            _ => return,
        };
        let name = *arrived.data;
        self.gather.heard(name, heard);
        // Before version 2 an output sends no `done`: each event is whole.
        if arrived.proxy.version() < 2 {
            self.gather.heard(name, Heard::Done);
        }
    }

    fn xdg_output_event(&mut self, arrived: Arrived<'_, ZxdgOutputV1, u32>) {
        let heard = match arrived.event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => Heard::Position { x, y },
            zxdg_output_v1::Event::LogicalSize { width, height } => Heard::Size { width, height },
            zxdg_output_v1::Event::Done => Heard::Done,
            _ => return,
        };
        self.gather.heard(*arrived.data, heard);
    }

    /// The manager sends no events.
    fn manager_event(&mut self, _: Arrived<'_, ZxdgOutputManagerV1, ()>) {}

    fn callback_event(&mut self, arrived: Arrived<'_, WlCallback, Mark>) {
        let wl_callback::Event::Done { .. } = arrived.event else {
            return;
        };
        match arrived.data {
            Mark::Globals => self.globals_known = true,
            Mark::Layout => {
                if let Some(layout) = self.gather.synced() {
                    self.settled = Some(layout);
                }
            }
        }
    }
}

/// Whether an output turned by `transform` has its mode's width upright.
fn turned(transform: WEnum<Transform>) -> bool {
    matches!(
        transform,
        WEnum::Value(
            Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
        )
    )
}

/// Whether a mode event is about the current mode.
fn current(flags: WEnum<wl_output::Mode>) -> bool {
    matches!(flags, WEnum::Value(flags) if flags.contains(wl_output::Mode::Current))
}

/// The connection, its queue and what its handlers keep.
pub(crate) struct Link {
    connection: Connection,
    queue: EventQueue<Client>,
    pub(crate) client: Client,
    /// The connection's socket, as the runtime waits on it.
    socket: tokio::net::UnixStream,
}

impl Link {
    /// Connects over `stream`, binds the outputs and reads the first
    /// layout; `None` while it is not a whole layout.
    pub(crate) async fn start(stream: UnixStream) -> Result<(Link, Option<Layout>), Missing> {
        let mut link = Link::over(stream)?;
        let queue = link.queue.handle();
        let registry = link.connection.display().get_registry(&queue, ());
        link.connection.display().sync(&queue, Mark::Globals);
        link.until(|client| client.globals_known).await?;
        link.client.bind_offered(&registry, &queue)?;
        link.until(|client| client.settled.is_some()).await?;
        let first = link.client.settled.take().flatten();
        Ok((link, first))
    }

    /// A connection over `stream`, nothing sent yet.
    pub(crate) fn over(stream: UnixStream) -> Result<Link, Missing> {
        let cannot = |err: std::io::Error| {
            eprintln!("auto-clicker: cannot use the Wayland display's socket: {err}");
            Missing::CannotConnect
        };
        stream.set_nonblocking(true).map_err(cannot)?;
        let watched = stream.try_clone().map_err(cannot)?;
        let socket = tokio::net::UnixStream::from_std(watched).map_err(cannot)?;
        let connection = Connection::from_socket(stream).map_err(|err| {
            eprintln!("auto-clicker: cannot use the Wayland display: {err}");
            Missing::CannotConnect
        })?;
        let queue = connection.new_event_queue();
        Ok(Link {
            connection,
            queue,
            client: Client::default(),
            socket,
        })
    }

    /// Runs until `done` holds after a dispatch.
    async fn until(&mut self, done: impl Fn(&Client) -> bool) -> Result<(), Missing> {
        loop {
            self.dispatch()?;
            if done(&self.client) {
                return Ok(());
            }
            self.wait().await?;
        }
    }

    /// Handles what has arrived, and asks for the round trip [`Gather`]
    /// wants.
    pub(crate) fn dispatch(&mut self) -> Result<(), Missing> {
        self.queue
            .dispatch_pending(&mut self.client)
            .map_err(dispatch_error)?;
        if self.client.gather.wants_sync() {
            self.connection
                .display()
                .sync(&self.queue.handle(), Mark::Layout);
        }
        Ok(())
    }

    /// Sends what is queued, then waits until something arrives and reads
    /// it.
    pub(crate) async fn wait(&mut self) -> Result<(), Missing> {
        self.flush().await?;
        let Some(read) = self.queue.prepare_read() else {
            // Events already arrived: the next dispatch handles them.
            return Ok(());
        };
        self.socket
            .readable()
            .await
            .map_err(|err| self.failed(err))?;
        let socket = &self.socket;
        // A read that would block clears the readiness and waits again.
        match socket.try_io(Interest::READABLE, || read_from(socket, read)) {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(err)) => Err(wayland_error(err)),
            Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(()),
            Err(err) => Err(self.failed(err)),
        }
    }

    async fn flush(&mut self) -> Result<(), Missing> {
        loop {
            self.socket
                .writable()
                .await
                .map_err(|err| self.failed(err))?;
            let connection = &self.connection;
            match self
                .socket
                .try_io(Interest::WRITABLE, || io_only(connection.flush()))
            {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(err)) => return Err(wayland_error(err)),
                Err(err) if err.kind() == ErrorKind::WouldBlock => {}
                Err(err) => return Err(self.failed(err)),
            }
        }
    }
}

impl Link {
    /// Why the connection ended with `err`: libwayland reports a protocol
    /// error as a failed read or write, and keeps the error itself.
    fn failed(&self, err: std::io::Error) -> Missing {
        match self.connection.protocol_error() {
            Some(protocol) => wayland_error(WaylandError::Protocol(protocol)),
            None => lost(err),
        }
    }
}

/// Reads what arrived on `socket`. libwayland, which a workspace build links
/// in for the settings window, reads nothing as no events rather than as a
/// read that would block, and a closed socket the same way, every time: so
/// a read of no events looks at the socket itself.
fn read_from(
    socket: &tokio::net::UnixStream,
    read: ReadEventsGuard,
) -> std::io::Result<Result<usize, WaylandError>> {
    match io_only(read.read())? {
        Ok(0) => match socket::waiting(socket) {
            Waiting::Nothing => Err(ErrorKind::WouldBlock.into()),
            Waiting::Closed => Err(ErrorKind::UnexpectedEof.into()),
            Waiting::Data => Ok(Ok(0)),
        },
        other => Ok(other),
    }
}

/// `result` with its socket errors on the outside, where the runtime
/// reads a read or write that would block; a protocol error inside.
fn io_only<T>(result: Result<T, WaylandError>) -> std::io::Result<Result<T, WaylandError>> {
    match result {
        Err(WaylandError::Io(err)) => Err(err),
        other => Ok(other),
    }
}

fn lost(err: std::io::Error) -> Missing {
    eprintln!("auto-clicker: lost the Wayland display: {err}");
    Missing::Lost
}

fn wayland_error(err: WaylandError) -> Missing {
    match err {
        WaylandError::Io(err) => lost(err),
        WaylandError::Protocol(err) => {
            eprintln!("auto-clicker: the Wayland display refused the connection: {err}");
            Missing::ProtocolError
        }
    }
}

fn dispatch_error(err: DispatchError) -> Missing {
    match err {
        DispatchError::Backend(err) => wayland_error(err),
        DispatchError::BadMessage {
            interface, opcode, ..
        } => {
            eprintln!(
                "auto-clicker: the Wayland display sent a malformed {interface} event ({opcode})"
            );
            Missing::ProtocolError
        }
    }
}

#[cfg(test)]
mod tests;
