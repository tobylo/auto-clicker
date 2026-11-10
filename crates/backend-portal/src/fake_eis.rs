//! A compositor's EIS side for tests: reis's server half over a socket
//! pair, offering what KWin offers (a relative pointer, an absolute pointer
//! with one region per output, and a keyboard) and recording what arrives.
//! It runs as a local task on the test's own runtime.

use std::cell::RefCell;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::rc::Rc;

use reis::PendingRequestResult;
use reis::eis::{self, device::DeviceType};
use reis::handshake::EisHandshaker;
use reis::request::{Connection, Device, DeviceCapability, EisRequest, EisRequestConverter, Seat};
use tokio::io::Interest;
use tokio::net::UnixStream as Readiness;
use tokio::sync::mpsc;

/// What the client sent, as the compositor reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Got {
    StartEmulating(&'static str),
    Button(u32, bool),
    Key(u32, bool),
    Motion(f32, f32),
    Absolute(f32, f32),
    Scroll(i32, i32),
    Frame(&'static str),
    Disconnect,
}

/// What the test makes the compositor do.
#[derive(Debug)]
pub(crate) enum Command {
    /// Ends the connection, as KWin does when the portal session closes.
    Disconnect,
    /// Pauses every device.
    Pause,
    /// Removes the absolute pointer, as on a monitor change.
    RemoveAbsolute,
    /// Closes the socket without a word.
    Close,
}

/// One output's region: x, y, width, height.
pub(crate) type Region = (u32, u32, u32, u32);

/// The test's handle on the fake compositor.
pub(crate) struct FakeEis {
    got: Rc<RefCell<Vec<Got>>>,
    commands: mpsc::UnboundedSender<Command>,
}

impl FakeEis {
    /// Starts the fake on a local task, with absolute regions `regions`;
    /// returns it and the client's end of the socket.
    pub(crate) fn start(regions: &[Region]) -> (FakeEis, OwnedFd) {
        let (server, client) = UnixStream::pair().unwrap();
        server.set_nonblocking(true).unwrap();
        let got = Rc::new(RefCell::new(Vec::new()));
        let (commands, inbox) = mpsc::unbounded_channel();
        // Readiness comes from a duplicate of the socket; the context reads.
        let readiness = Readiness::from_std(server.try_clone().unwrap()).unwrap();
        let context = eis::Context::new(server).unwrap();
        let server = Server {
            handshaker: Some(EisHandshaker::new(&context, 1)),
            context,
            regions: regions.to_vec(),
            got: got.clone(),
            converter: None,
            _seat: None,
            devices: Devices::default(),
        };
        tokio::task::spawn_local(serve(server, readiness, inbox));
        (FakeEis { got, commands }, client.into())
    }

    /// Everything received so far.
    pub(crate) fn got(&self) -> Vec<Got> {
        self.got.borrow().clone()
    }

    /// Everything received so far, without the emulation starts and frames.
    pub(crate) fn events(&self) -> Vec<Got> {
        self.got()
            .into_iter()
            .filter(|got| !matches!(got, Got::StartEmulating(_) | Got::Frame(_)))
            .collect()
    }

    pub(crate) fn command(&self, command: Command) {
        self.commands.send(command).unwrap();
    }
}

#[derive(Default)]
struct Devices {
    all: Vec<(&'static str, Device)>,
}

impl Devices {
    fn remove(&mut self, removed: &str) {
        self.all.retain(|(name, device)| {
            let keep = *name != removed;
            if !keep {
                device.remove();
            }
            keep
        });
    }

    fn name(&self, device: &Device) -> &'static str {
        self.all
            .iter()
            .find(|(_, d)| d == device)
            .map_or("unknown", |(name, _)| name)
    }
}

/// Whether the fake goes on after an event.
enum Next {
    Continue,
    Stop,
}

/// The compositor's side of one connection.
struct Server {
    context: eis::Context,
    regions: Vec<Region>,
    got: Rc<RefCell<Vec<Got>>>,
    /// Until the handshake ends.
    handshaker: Option<EisHandshaker>,
    /// After the handshake.
    converter: Option<EisRequestConverter>,
    /// Kept for the connection's lifetime, as a compositor keeps its seat.
    _seat: Option<Seat>,
    devices: Devices,
}

async fn serve(
    mut server: Server,
    socket: Readiness,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    loop {
        let next = tokio::select! {
            ready = socket.readable() => {
                ready.unwrap();
                let next = server.read();
                // The context read until it would block: clear the readiness.
                let _ = socket.try_io(Interest::READABLE, || Err::<(), _>(io::ErrorKind::WouldBlock.into()));
                next
            }
            command = commands.recv() => match command {
                Some(command) => server.command(command),
                None => Next::Stop,
            },
        };
        let _ = server.context.flush();
        if matches!(next, Next::Stop) {
            return;
        }
    }
}

impl Server {
    /// Reads and handles what the client sent.
    fn read(&mut self) -> Next {
        match self.context.read() {
            Ok(0) => return Next::Stop,
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => return Next::Stop,
        }
        while let Some(result) = self.context.pending_request() {
            if let PendingRequestResult::Request(request) = result
                && let Next::Stop = self.request(request)
            {
                return Next::Stop;
            }
        }
        Next::Continue
    }

    fn request(&mut self, request: eis::Request) -> Next {
        let Some(converter) = &mut self.converter else {
            self.handshake(request);
            return Next::Continue;
        };
        converter.handle_request(request).unwrap();
        let connection = converter.handle().clone();
        let requests: Vec<EisRequest> = std::iter::from_fn(|| converter.next_request()).collect();
        for request in requests {
            if let Next::Stop = self.handle(request, &connection) {
                return Next::Stop;
            }
        }
        Next::Continue
    }

    fn handshake(&mut self, request: eis::Request) {
        let handshaker = self.handshaker.as_mut().unwrap();
        let Some(resp) = handshaker.handle_request(request).unwrap() else {
            return;
        };
        let connected = EisRequestConverter::new(&self.context, resp, 1);
        self._seat = Some(connected.handle().add_seat(
            Some("default"),
            DeviceCapability::Pointer
                | DeviceCapability::PointerAbsolute
                | DeviceCapability::Keyboard
                | DeviceCapability::Button
                | DeviceCapability::Scroll,
        ));
        self.converter = Some(connected);
        self.handshaker = None;
    }

    /// Records one request; binds add the devices.
    fn handle(&mut self, request: EisRequest, connection: &Connection) -> Next {
        let devices = &self.devices;
        let event = match request {
            EisRequest::Disconnect => Got::Disconnect,
            EisRequest::Bind(bind) if devices.all.is_empty() => {
                add_devices(&bind.seat, &self.regions, &mut self.devices);
                let _ = connection.flush();
                return Next::Continue;
            }
            EisRequest::DeviceStartEmulating(start) => {
                Got::StartEmulating(devices.name(&start.device))
            }
            EisRequest::Button(b) => {
                Got::Button(b.button, b.state == eis::button::ButtonState::Press)
            }
            EisRequest::KeyboardKey(k) => {
                Got::Key(k.key, k.state == eis::keyboard::KeyState::Press)
            }
            EisRequest::PointerMotion(m) => Got::Motion(m.dx, m.dy),
            EisRequest::PointerMotionAbsolute(m) => Got::Absolute(m.dx_absolute, m.dy_absolute),
            EisRequest::ScrollDiscrete(s) => Got::Scroll(s.discrete_dx, s.discrete_dy),
            EisRequest::Frame(frame) => Got::Frame(devices.name(&frame.device)),
            _ => return Next::Continue,
        };
        self.got.borrow_mut().push(event);
        if event == Got::Disconnect {
            Next::Stop
        } else {
            Next::Continue
        }
    }

    /// Does what the test asks.
    fn command(&mut self, command: Command) -> Next {
        let Some(converter) = &self.converter else {
            // Only closing works before the handshake ends.
            return match command {
                Command::Close => Next::Stop,
                _ => Next::Continue,
            };
        };
        match command {
            Command::Disconnect => {
                converter.handle().disconnected(
                    eis::connection::DisconnectReason::Disconnected,
                    Some("the session closed"),
                );
                let _ = self.context.flush();
                return Next::Stop;
            }
            Command::Pause => self.devices.all.iter().for_each(|(_, d)| d.paused()),
            Command::RemoveAbsolute => self.devices.remove("absolute"),
            Command::Close => return Next::Stop,
        }
        Next::Continue
    }
}

/// KWin's three devices, resumed.
fn add_devices(seat: &Seat, regions: &[Region], devices: &mut Devices) {
    let pointer = seat.add_device(
        Some("pointer"),
        DeviceType::Virtual,
        DeviceCapability::Pointer | DeviceCapability::Button | DeviceCapability::Scroll,
        |_| {},
    );
    let absolute = seat.add_device(
        Some("absolute"),
        DeviceType::Virtual,
        DeviceCapability::PointerAbsolute | DeviceCapability::Button | DeviceCapability::Scroll,
        |device| {
            for &(x, y, width, height) in regions {
                device.device().region(x, y, width, height, 1.0);
            }
        },
    );
    let keyboard = seat.add_device(
        Some("keyboard"),
        DeviceType::Virtual,
        DeviceCapability::Keyboard.into(),
        |_| {},
    );
    for (name, device) in [
        ("pointer", pointer),
        ("absolute", absolute),
        ("keyboard", keyboard),
    ] {
        device.resumed();
        devices.all.push((name, device));
    }
}
