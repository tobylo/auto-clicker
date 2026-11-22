//! A fake compositor on the other end of a socket, speaking the wire
//! protocol by hand: the layout's tests, the cursor sessions' and the
//! on-screen display's share it.

mod shell;

pub(crate) use shell::{COMPOSITOR, FRACTIONS, OVERLAY, SHELL, SHM, Shell, VIEWPORTER};

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use auto_clicker_core::recording::{Layout, Output, Rect};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Longer than any step of a test takes; a test that waits it out fails.
pub(crate) const BOUND: Duration = Duration::from_secs(5);

pub(crate) const DISPLAY: u32 = 1;
pub(crate) const REGISTRY: u32 = 2;

/// A global's name, as the fake offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Name(pub(crate) u32);

pub(crate) const MANAGER: Name = Name(1);
pub(crate) const SEAT: Name = Name(2);
pub(crate) const CAPTURE: Name = Name(3);
pub(crate) const SOURCES: Name = Name(4);
pub(crate) const A: Name = Name(10);
pub(crate) const B: Name = Name(11);
pub(crate) const C: Name = Name(12);

/// What the fake offers.
pub(crate) enum Offer {
    Manager,
    /// The manager at version 2: each xdg-output sends its own `done`.
    OldManager,
    Output(Name),
    /// A `wl_output` at an older version: 1 sends no `done`, 2 does.
    OldOutput(Name, u32),
    /// What cursor sessions need: a seat and both managers.
    Seat,
    CaptureManager,
    SourceManager,
    /// What the on-screen display needs: surfaces at a version, shared
    /// memory and layer-shell; and the viewport and fractional scale.
    Compositor(u32),
    Shm,
    LayerShell,
    Viewporter,
    Fractions,
}

impl Offer {
    /// Everything the on-screen display uses.
    pub(crate) fn display() -> [Offer; 5] {
        [
            Offer::Compositor(6),
            Offer::Shm,
            Offer::LayerShell,
            Offer::Viewporter,
            Offer::Fractions,
        ]
    }
}

/// An output's logical place and size, as xdg-output sends it.
#[derive(Clone, Copy)]
pub(crate) struct Place {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl Place {
    /// The same, moved by nothing: a layout's rectangle at scale 1.
    pub(crate) fn output(self) -> Output {
        let rect = Rect {
            x: u32::try_from(self.x).unwrap(),
            y: u32::try_from(self.y).unwrap(),
            width: u32::try_from(self.width).unwrap(),
            height: u32::try_from(self.height).unwrap(),
        };
        Output::new(rect, 1000).unwrap()
    }
}

pub(crate) const LEFT: Place = Place {
    x: 0,
    y: 0,
    width: 2048,
    height: 1280,
};
pub(crate) const BELOW: Place = Place {
    x: 0,
    y: 1280,
    width: 1920,
    height: 1080,
};
pub(crate) const RIGHT: Place = Place {
    x: 2048,
    y: 0,
    width: 3440,
    height: 1440,
};

/// A message, either way: its object, opcode and arguments in words.
#[derive(Debug)]
pub(crate) struct Message {
    pub(crate) object: u32,
    pub(crate) opcode: u16,
    pub(crate) words: Vec<u32>,
}

impl Message {
    pub(crate) fn bytes(&self) -> Vec<u8> {
        let size = u32::try_from(8 + 4 * self.words.len()).unwrap();
        let header = [self.object, (size << 16) | u32::from(self.opcode)];
        header
            .iter()
            .chain(&self.words)
            .flat_map(|word| word.to_ne_bytes())
            .collect()
    }
}

/// A round trip the client asked for: its callback's id.
#[derive(Clone, Copy)]
pub(crate) struct Callback(pub(crate) u32);

/// A bound output's `wl_output` and xdg-output ids.
#[derive(Clone, Copy)]
pub(crate) struct Ids {
    pub(crate) output: u32,
    pub(crate) xdg: u32,
}

/// A string argument in words: its length with the NUL, then the bytes,
/// padded.
pub(crate) fn string(text: &str) -> Vec<u32> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    let length = u32::try_from(bytes.len()).unwrap();
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    let words = bytes
        .chunks(4)
        .map(|chunk| u32::from_ne_bytes(chunk.try_into().unwrap()));
    std::iter::once(length).chain(words).collect()
}

/// The fake compositor's end, and the ids the client gave its objects.
pub(crate) struct Fake {
    pub(crate) stream: tokio::net::UnixStream,
    pub(crate) outputs: Vec<(Name, Ids)>,
    /// Each request read, as its object and opcode.
    pub(crate) seen: Vec<(u32, u16)>,
    /// Each global bound, with the id the client gave it.
    pub(crate) bound: Vec<(Name, u32)>,
    /// Each output's capture source: its id and the output's.
    pub(crate) sources: Vec<(u32, u32)>,
    /// Each cursor session: its id and its source's.
    pub(crate) sessions: Vec<(u32, u32)>,
    /// Whether [`Fake::start_with_two`] offers what cursor sessions need.
    pub(crate) cursors: bool,
    /// Whether [`Fake::start_with_two`] offers what the on-screen display
    /// needs.
    pub(crate) display: bool,
    /// What the on-screen display made.
    pub(crate) shell: Shell,
}

impl Fake {
    /// A connected pair: the client's end and the fake's.
    pub(crate) fn pair() -> (UnixStream, Fake) {
        let (ours, theirs) = UnixStream::pair().unwrap();
        theirs.set_nonblocking(true).unwrap();
        let fake = Fake::over(tokio::net::UnixStream::from_std(theirs).unwrap());
        (ours, fake)
    }

    /// The fake on its end of `stream`.
    pub(crate) fn over(stream: tokio::net::UnixStream) -> Fake {
        Fake {
            stream,
            outputs: Vec::new(),
            seen: Vec::new(),
            bound: Vec::new(),
            sources: Vec::new(),
            sessions: Vec::new(),
            cursors: false,
            display: false,
            shell: Shell::default(),
        }
    }

    /// Whether the client closed its end: nothing more to read.
    /// A client that closes with events unread resets the connection.
    pub(crate) async fn closed(&mut self) -> bool {
        let mut rest = Vec::new();
        match self.stream.read_to_end(&mut rest).await {
            Ok(_) => true,
            Err(err) => err.kind() == std::io::ErrorKind::ConnectionReset,
        }
    }

    pub(crate) async fn send(&mut self, messages: Vec<Message>) {
        let bytes: Vec<u8> = messages.iter().flat_map(Message::bytes).collect();
        self.stream.write_all(&bytes).await.unwrap();
    }

    pub(crate) async fn request(&mut self) -> Message {
        let mut header = [0u8; 8];
        self.stream.read_exact(&mut header).await.unwrap();
        let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
        let second = u32::from_ne_bytes(header[4..].try_into().unwrap());
        let size = usize::try_from(second >> 16).unwrap();
        let mut body = vec![0u8; size - 8];
        self.stream.read_exact(&mut body).await.unwrap();
        let words = body
            .chunks(4)
            .map(|chunk| u32::from_ne_bytes(chunk.try_into().unwrap()))
            .collect();
        Message {
            object,
            opcode: u16::try_from(second & 0xffff).unwrap(),
            words,
        }
    }

    /// Reads requests up to the client's next `wl_display.sync`, noting the
    /// outputs it binds.
    pub(crate) async fn until_sync(&mut self) -> Callback {
        loop {
            if let Some(callback) = self.read_one().await {
                return callback;
            }
        }
    }

    /// Reads requests until the client has opened `count` cursor sessions
    /// in all.
    pub(crate) async fn until_sessions(&mut self, count: usize) {
        while self.sessions.len() < count {
            self.read_one().await;
        }
    }

    /// The id the client gave the global `name`.
    fn id_of(&self, name: Name) -> Option<u32> {
        let found = self.bound.iter().rev().find(|(bound, _)| *bound == name);
        found.map(|&(_, id)| id)
    }

    /// Reads requests, answering every round trip, until `done` holds.
    pub(crate) async fn until(&mut self, done: impl Fn(&Fake) -> bool) {
        while !done(self) {
            if let Some(callback) = self.read_one().await {
                self.answer(callback).await;
            }
        }
    }

    /// Reads one request, noting what it binds or makes; the callback of a
    /// `wl_display.sync`.
    async fn read_one(&mut self) -> Option<Callback> {
        let request = self.request().await;
        self.seen.push((request.object, request.opcode));
        let global = self
            .bound
            .iter()
            .rev()
            .find(|(_, id)| *id == request.object)
            .map(|&(name, _)| name);
        self.shell.note(&request, global);
        let words = &request.words;
        let on = |name| self.id_of(name) == Some(request.object);
        match (request.object, request.opcode) {
            (DISPLAY, 0) => return Some(Callback(words[0])),
            // wl_registry.bind: name, interface, version, new id.
            (REGISTRY, 0) => self.bound.push((Name(words[0]), *words.last().unwrap())),
            // zxdg_output_manager_v1.get_xdg_output: new id, output.
            (_, 1) if on(MANAGER) => {
                let output = self.bound.iter().find(|(_, id)| *id == words[1]);
                if let Some(&(name, output)) = output {
                    let xdg = words[0];
                    self.outputs.push((name, Ids { output, xdg }));
                }
            }
            // ext_output_image_capture_source_manager_v1.create_source:
            // new id, output.
            (_, 0) if on(SOURCES) => self.sources.push((words[0], words[1])),
            // ext_image_copy_capture_manager_v1.create_pointer_cursor_session:
            // new id, source, pointer.
            (_, 1) if on(CAPTURE) => self.sessions.push((words[0], words[1])),
            _ => {}
        }
        None
    }

    /// Answers the client's next round trip.
    pub(crate) async fn round_trip(&mut self) {
        let callback = self.until_sync().await;
        self.answer(callback).await;
    }

    /// Answers the round trip `callback`.
    pub(crate) async fn answer(&mut self, Callback(id): Callback) {
        let done = Message {
            object: id,
            opcode: 0,
            words: vec![0],
        };
        let delete_id = Message {
            object: DISPLAY,
            opcode: 1,
            words: vec![id],
        };
        self.send(vec![done, delete_id]).await;
    }

    pub(crate) async fn offer(&mut self, offer: Offer) {
        let (Name(name), interface, version) = match offer {
            Offer::Manager => (MANAGER, "zxdg_output_manager_v1", 3),
            Offer::OldManager => (MANAGER, "zxdg_output_manager_v1", 2),
            Offer::Output(name) => (name, "wl_output", 4),
            Offer::OldOutput(name, version) => (name, "wl_output", version),
            Offer::Seat => (SEAT, "wl_seat", 9),
            Offer::CaptureManager => (CAPTURE, "ext_image_copy_capture_manager_v1", 1),
            Offer::SourceManager => (SOURCES, "ext_output_image_capture_source_manager_v1", 1),
            Offer::Compositor(version) => (COMPOSITOR, "wl_compositor", version),
            Offer::Shm => (SHM, "wl_shm", 1),
            Offer::LayerShell => (SHELL, "zwlr_layer_shell_v1", 4),
            Offer::Viewporter => (VIEWPORTER, "wp_viewporter", 1),
            Offer::Fractions => (FRACTIONS, "wp_fractional_scale_manager_v1", 1),
        };
        let words = std::iter::once(name)
            .chain(string(interface))
            .chain([version])
            .collect();
        let global = Message {
            object: REGISTRY,
            opcode: 0,
            words,
        };
        self.send(vec![global]).await;
    }

    pub(crate) fn ids(&self, name: Name) -> Ids {
        let found = self.outputs.iter().find(|(bound, _)| *bound == name);
        found.unwrap().1
    }

    /// The output `name` at `place`, at scale 1 and 60 Hz.
    pub(crate) async fn screen(&mut self, name: Name, place: Place) {
        self.mode(name, place, 60_000).await;
    }

    /// The output `name` at `place`, its mode at scale 1 refreshing at
    /// `refresh` mHz, and its `done` on `wl_output` (xdg-output 3).
    pub(crate) async fn mode(&mut self, name: Name, place: Place, refresh: u32) {
        let Ids { output, xdg } = self.ids(name);
        let Place {
            x,
            y,
            width,
            height,
        } = place;
        let [x, y, width, height] = [x, y, width, height].map(i32::cast_unsigned);
        let message = |object, opcode, words| Message {
            object,
            opcode,
            words,
        };
        self.send(vec![
            message(xdg, 0, vec![x, y]),
            message(xdg, 1, vec![width, height]),
            message(output, 1, vec![1, width, height, refresh]),
            message(output, 2, Vec::new()),
        ])
        .await;
    }

    /// Offers the manager and outputs `A` and `B`, answers the first round
    /// trip, and gives both outputs side by side once bound. Their updates
    /// arrive before the answer to the round trip after the binds, so the
    /// client asks once more, and that answer settles the layout.
    pub(crate) async fn start_with_two(&mut self) {
        let callback = self.until_sync().await;
        self.offer(Offer::Manager).await;
        self.offer(Offer::Output(A)).await;
        self.offer(Offer::Output(B)).await;
        for offer in self.extras() {
            self.offer(offer).await;
        }
        self.answer(callback).await;
        let callback = self.until_sync().await;
        self.screen(A, LEFT).await;
        self.screen(B, RIGHT).await;
        self.answer(callback).await;
        self.round_trip().await;
    }
}

impl Fake {
    /// What [`Fake::start_with_two`] offers besides the outputs: what
    /// cursor sessions need, and the on-screen display, as set.
    fn extras(&self) -> Vec<Offer> {
        let cursors = [Offer::Seat, Offer::CaptureManager, Offer::SourceManager];
        let cursors = cursors.into_iter().filter(|_| self.cursors);
        let display = Offer::display().into_iter().filter(|_| self.display);
        cursors.chain(display).collect()
    }

    /// Answers the round trip after the seat was bound, the seat telling
    /// first that it has a keyboard and whether it has a pointer.
    pub(crate) async fn seat(&mut self, pointer: bool) {
        let callback = self.until_sync().await;
        let capabilities = self.capabilities(pointer);
        self.send(vec![capabilities]).await;
        self.answer(callback).await;
    }

    /// The seat telling what it has: a keyboard, as on a desktop, and a
    /// pointer when `pointer` holds.
    pub(crate) fn capabilities(&self, pointer: bool) -> Message {
        Message {
            object: self.id_of(SEAT).unwrap(),
            opcode: 0,
            words: vec![u32::from(pointer) | 2],
        }
    }

    /// The cursor session on the output `name`.
    pub(crate) fn session(&self, name: Name) -> u32 {
        let output = self.ids(name).output;
        let (source, _) = *self
            .sources
            .iter()
            .rev()
            .find(|(_, of)| *of == output)
            .unwrap();
        let (session, _) = *self
            .sessions
            .iter()
            .rev()
            .find(|(_, of)| *of == source)
            .unwrap();
        session
    }

    /// The pointer at (`x`, `y`) physical pixels on the output `name`, as
    /// its cursor session tells it.
    pub(crate) fn position(&self, name: Name, x: i32, y: i32) -> Message {
        Message {
            object: self.session(name),
            opcode: 2,
            words: vec![x.cast_unsigned(), y.cast_unsigned()],
        }
    }
}

/// `wl_registry.global_remove` for `name`.
pub(crate) fn removed(Name(name): Name) -> Message {
    Message {
        object: REGISTRY,
        opcode: 1,
        words: vec![name],
    }
}

pub(crate) fn layout(places: &[Place]) -> Option<Layout> {
    let outputs = places.iter().map(|place| place.output()).collect();
    Some(Layout::new(outputs).unwrap())
}

/// A listening socket standing in for the display, at a path of its own.
pub(crate) struct Display {
    pub(crate) listener: tokio::net::UnixListener,
    pub(crate) path: PathBuf,
}

impl Display {
    pub(crate) fn new() -> Display {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "auto-clicker-display-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        Display { listener, path }
    }

    /// The fake on the next connection made to the display.
    pub(crate) async fn accept(&self) -> Fake {
        let (stream, _) = self.listener.accept().await.unwrap();
        Fake::over(stream)
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
