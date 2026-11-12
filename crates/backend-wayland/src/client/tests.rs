//! The connection against a fake compositor on the other end of a socket
//! pair, speaking the wire protocol by hand: what is published when, and
//! how the connection ends.

use std::cell::Cell;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;

use auto_clicker_core::controller::LayoutSignal;
use auto_clicker_core::recording::{Output, Rect};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::LocalSet;
use tokio::time::timeout;

use super::*;
use crate::Wayland;

/// Longer than any step of a test takes; a test that waits it out fails.
const BOUND: Duration = Duration::from_secs(5);

const DISPLAY: u32 = 1;
const REGISTRY: u32 = 2;

/// A global's name, as the fake offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Name(u32);

const MANAGER: Name = Name(1);
const A: Name = Name(10);
const B: Name = Name(11);
const C: Name = Name(12);

/// What the fake offers.
enum Offer {
    Manager,
    Output(Name),
}

/// An output's logical place and size, as xdg-output sends it.
#[derive(Clone, Copy)]
struct Place {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Place {
    /// The same, moved by nothing: a layout's rectangle at scale 1.
    fn output(self) -> Output {
        let rect = Rect {
            x: u32::try_from(self.x).unwrap(),
            y: u32::try_from(self.y).unwrap(),
            width: u32::try_from(self.width).unwrap(),
            height: u32::try_from(self.height).unwrap(),
        };
        Output::new(rect, 1000).unwrap()
    }
}

const LEFT: Place = Place {
    x: 0,
    y: 0,
    width: 2048,
    height: 1280,
};
const RIGHT: Place = Place {
    x: 2048,
    y: 0,
    width: 3440,
    height: 1440,
};

/// A message, either way: its object, opcode and arguments in words.
#[derive(Debug)]
struct Message {
    object: u32,
    opcode: u16,
    words: Vec<u32>,
}

impl Message {
    fn bytes(&self) -> Vec<u8> {
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
struct Callback(u32);

/// A bound output's `wl_output` and xdg-output ids.
#[derive(Clone, Copy)]
struct Ids {
    output: u32,
    xdg: u32,
}

/// A string argument in words: its length with the NUL, then the bytes,
/// padded.
fn string(text: &str) -> Vec<u32> {
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
struct Fake {
    stream: tokio::net::UnixStream,
    outputs: Vec<(Name, Ids)>,
    /// Each request read, as its object and opcode.
    seen: Vec<(u32, u16)>,
}

impl Fake {
    /// A connected pair: the client's end and the fake's.
    fn pair() -> (UnixStream, Fake) {
        let (ours, theirs) = UnixStream::pair().unwrap();
        theirs.set_nonblocking(true).unwrap();
        let fake = Fake {
            stream: tokio::net::UnixStream::from_std(theirs).unwrap(),
            outputs: Vec::new(),
            seen: Vec::new(),
        };
        (ours, fake)
    }

    async fn send(&mut self, messages: Vec<Message>) {
        let bytes: Vec<u8> = messages.iter().flat_map(Message::bytes).collect();
        self.stream.write_all(&bytes).await.unwrap();
    }

    async fn request(&mut self) -> Message {
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
    async fn until_sync(&mut self) -> Callback {
        let mut bound: Vec<(Name, u32)> = Vec::new();
        loop {
            let request = self.request().await;
            self.seen.push((request.object, request.opcode));
            let words = &request.words;
            match (request.object, request.opcode) {
                (DISPLAY, 0) => return Callback(words[0]),
                // wl_registry.bind: name, interface, version, new id.
                (REGISTRY, 0) if Name(words[0]) != MANAGER => {
                    bound.push((Name(words[0]), *words.last().unwrap()));
                }
                // zxdg_output_manager_v1.get_xdg_output: new id, output.
                (_, 1) => {
                    if let Some(&(name, output)) = bound.iter().find(|(_, id)| *id == words[1]) {
                        let xdg = words[0];
                        self.outputs.push((name, Ids { output, xdg }));
                    }
                }
                _ => {}
            }
        }
    }

    /// Answers the round trip `callback`.
    async fn answer(&mut self, Callback(id): Callback) {
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

    async fn offer(&mut self, offer: Offer) {
        let (Name(name), interface, version) = match offer {
            Offer::Manager => (MANAGER, "zxdg_output_manager_v1", 3),
            Offer::Output(name) => (name, "wl_output", 4),
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

    fn ids(&self, name: Name) -> Ids {
        let found = self.outputs.iter().find(|(bound, _)| *bound == name);
        found.unwrap().1
    }

    /// The output `name` at `place`, at scale 1 and 60 Hz.
    async fn screen(&mut self, name: Name, place: Place) {
        self.mode(name, place, 60_000).await;
    }

    /// The output `name` at `place`, its mode at scale 1 refreshing at
    /// `refresh` mHz, and its `done` on `wl_output` (xdg-output 3).
    async fn mode(&mut self, name: Name, place: Place, refresh: u32) {
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
    async fn start_with_two(&mut self) {
        let callback = self.until_sync().await;
        self.offer(Offer::Manager).await;
        self.offer(Offer::Output(A)).await;
        self.offer(Offer::Output(B)).await;
        self.answer(callback).await;
        let callback = self.until_sync().await;
        self.screen(A, LEFT).await;
        self.screen(B, RIGHT).await;
        self.answer(callback).await;
        let callback = self.until_sync().await;
        self.answer(callback).await;
    }
}

/// `wl_registry.global_remove` for `name`.
fn removed(Name(name): Name) -> Message {
    Message {
        object: REGISTRY,
        opcode: 1,
        words: vec![name],
    }
}

fn layout(places: &[Place]) -> Option<Layout> {
    let outputs = places.iter().map(|place| place.output()).collect();
    Some(Layout::new(outputs).unwrap())
}

/// A started connection's handle, watched on `local`, and how often it
/// fired; the watcher's task.
struct Watched {
    wayland: Wayland,
    fired: Rc<Cell<usize>>,
    watch: tokio::task::JoinHandle<()>,
}

/// Starts a connection on two outputs and watches it on `local`.
async fn watched(local: &LocalSet, fake: &mut Fake, link: UnixStream) -> Watched {
    let (started, ()) = timeout(BOUND, async {
        tokio::join!(Link::start(link), fake.start_with_two())
    })
    .await
    .unwrap();
    let (link, first) = started.unwrap();
    let wayland = Wayland::of(NonZeroU32::new(std::process::id()), first, Ok(link));
    let changed = LayoutSignal::default();
    let fired = Rc::new(Cell::new(0));
    let seen = Rc::clone(&fired);
    let signal = changed.clone();
    local.spawn_local(async move {
        loop {
            signal.fired().await;
            seen.set(seen.get() + 1);
        }
    });
    let watch = local.spawn_local(wayland.watch(changed));
    Watched {
        wayland,
        fired,
        watch,
    }
}

/// Lets every ready task run.
async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// Waits until the signal fired.
async fn fired(watched: &Watched) {
    while watched.fired.get() == 0 {
        settle().await;
    }
}

#[tokio::test]
async fn two_outputs_give_one_whole_layout_at_start() {
    let (link, mut fake) = Fake::pair();
    let (started, ()) = timeout(BOUND, async {
        tokio::join!(Link::start(link), fake.start_with_two())
    })
    .await
    .unwrap();
    let (_, first) = started.unwrap();
    assert_eq!(first, layout(&[LEFT, RIGHT]));
}

#[tokio::test]
async fn a_display_without_xdg_output_gives_no_layout_and_says_why() {
    let (link, mut fake) = Fake::pair();
    let serve = async {
        let callback = fake.until_sync().await;
        fake.offer(Offer::Output(A)).await;
        fake.answer(callback).await;
    };
    let (started, ()) = timeout(BOUND, async { tokio::join!(Link::start(link), serve) })
        .await
        .unwrap();
    assert_eq!(started.err(), Some(Missing::NoXdgOutput));
}

#[tokio::test]
async fn globals_withdrawn_before_they_were_bound_are_never_bound() {
    let (link, mut fake) = Fake::pair();
    let serve = async {
        let callback = fake.until_sync().await;
        fake.offer(Offer::Manager).await;
        fake.offer(Offer::Output(A)).await;
        fake.offer(Offer::Output(B)).await;
        fake.send(vec![removed(B)]).await;
        fake.answer(callback).await;
        let callback = fake.until_sync().await;
        let bound: Vec<Name> = fake.outputs.iter().map(|(name, _)| *name).collect();
        assert_eq!(bound, [A], "bound a withdrawn output");
        fake.screen(A, LEFT).await;
        fake.answer(callback).await;
        let callback = fake.until_sync().await;
        fake.answer(callback).await;
    };
    let (started, ()) = timeout(BOUND, async { tokio::join!(Link::start(link), serve) })
        .await
        .unwrap();
    assert_eq!(started.unwrap().1, layout(&[LEFT]));

    // The manager withdrawn before it was bound: no xdg-output.
    let (link, mut fake) = Fake::pair();
    let serve = async {
        let callback = fake.until_sync().await;
        fake.offer(Offer::Manager).await;
        fake.offer(Offer::Output(A)).await;
        fake.send(vec![removed(MANAGER)]).await;
        fake.answer(callback).await;
    };
    let (started, ()) = timeout(BOUND, async { tokio::join!(Link::start(link), serve) })
        .await
        .unwrap();
    assert_eq!(started.err(), Some(Missing::NoXdgOutput));
}

#[tokio::test]
async fn the_mode_turn_and_scale_reach_the_layout_and_fire_nothing() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (link, mut fake) = Fake::pair();
            let watched = watched(&local, &mut fake, link).await;
            timeout(BOUND, async {
                let output = fake.ids(A).output;
                let message = |opcode, words| Message {
                    object: output,
                    opcode,
                    words,
                };
                // Turned a quarter (transform 90), its current mode 1600 x
                // 2560 and another it could take, at whole scale 2: the
                // same logical 2048 x 1280 at 1.25.
                let geometry = [0, 0, 300, 200, 0]
                    .into_iter()
                    .chain(string("make"))
                    .chain(string("model"))
                    .chain([1])
                    .collect();
                fake.send(vec![
                    message(0, geometry),
                    message(1, vec![1, 1600, 2560, 60_000]),
                    message(1, vec![0, 3840, 2160, 60_000]),
                    message(3, vec![2]),
                    message(2, Vec::new()),
                ])
                .await;
                let callback = fake.until_sync().await;
                fake.answer(callback).await;
                // A removal after it shows the client handled the answer.
                fake.send(vec![removed(B)]).await;
                let callback = fake.until_sync().await;
                fake.answer(callback).await;
                fired(&watched).await;
            })
            .await
            .unwrap();
            settle().await;
            assert_eq!(watched.fired.get(), 1, "a new scale alone fired");
            let turned = Output::new(LEFT.output().rect(), 1250).unwrap();
            assert_eq!(
                watched.wayland.layout(),
                Some(Layout::new(vec![turned]).unwrap())
            );
        })
        .await;
}

#[tokio::test]
async fn an_update_to_two_outputs_interleaved_publishes_once_whole() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (link, mut fake) = Fake::pair();
            let watched = watched(&local, &mut fake, link).await;
            let left = Place {
                width: 1707,
                height: 1067,
                ..LEFT
            };
            let right = Place { x: 1707, ..RIGHT };
            timeout(BOUND, async {
                // A's half: the client asks for a round trip.
                fake.screen(A, left).await;
                let callback = fake.until_sync().await;
                // B's half arrives before the round trip's answer.
                fake.screen(B, right).await;
                fake.answer(callback).await;
                // The client saw news behind the answer and asks again,
                // with the old layout still published.
                let again = fake.until_sync().await;
                settle().await;
                let published = watched.wayland.layout();
                assert_eq!(
                    published,
                    layout(&[LEFT, RIGHT]),
                    "a half layout was published"
                );
                assert_eq!(watched.fired.get(), 0);
                fake.answer(again).await;
                fired(&watched).await;
            })
            .await
            .unwrap();
            assert_eq!(watched.wayland.layout(), layout(&[left, right]));
            settle().await;
            assert_eq!(watched.fired.get(), 1);
            assert_eq!(watched.wayland.missing(), None);
        })
        .await;
}

#[tokio::test]
async fn an_update_that_moves_nothing_fires_nothing() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (link, mut fake) = Fake::pair();
            let watched = watched(&local, &mut fake, link).await;
            timeout(BOUND, async {
                // A new refresh rate on A, and its done: the same rectangle.
                fake.mode(A, LEFT, 144_000).await;
                let callback = fake.until_sync().await;
                fake.answer(callback).await;
                // A removal after it shows the client handled the answer.
                fake.send(vec![removed(B)]).await;
                let callback = fake.until_sync().await;
                fake.answer(callback).await;
                fired(&watched).await;
            })
            .await
            .unwrap();
            settle().await;
            assert_eq!(watched.fired.get(), 1, "the refresh rate fired too");
            assert_eq!(watched.wayland.layout(), layout(&[LEFT]));
            // B's objects were released: its xdg-output destroyed, its
            // output (version 3 and later) released.
            let Ids { output, xdg } = fake.ids(B);
            assert!(fake.seen.contains(&(xdg, 0)), "B's xdg-output kept");
            assert!(fake.seen.contains(&(output, 0)), "B's output kept");
        })
        .await;
}

/// Starts with two outputs, watches, lets `end` end the connection from
/// the fake's side, and returns the handle and how often it fired.
async fn ended_by(
    end: impl AsyncFnOnce(Fake) + 'static,
) -> (Option<Layout>, Option<Missing>, usize) {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (link, mut fake) = Fake::pair();
            let watched = watched(&local, &mut fake, link).await;
            local.spawn_local(async move { end(fake).await });
            timeout(BOUND, watched.watch).await.unwrap().unwrap();
            settle().await;
            let Watched { wayland, fired, .. } = watched;
            (wayland.layout(), wayland.missing(), fired.get())
        })
        .await
}

#[tokio::test]
async fn a_lost_connection_makes_the_layout_unknown_for_good() {
    let (layout, missing, fired) = ended_by(async |fake| drop(fake)).await;
    assert_eq!((layout, missing, fired), (None, Some(Missing::Lost), 1));
}

#[tokio::test]
async fn a_connection_that_refuses_a_request_is_lost() {
    // The fake stops reading but still sends: an update the client answers
    // with a round trip, whose request is what finds it gone.
    let (layout, missing, fired) = ended_by(async |mut fake| {
        let stream = fake.stream.into_std().unwrap();
        stream.shutdown(std::net::Shutdown::Read).unwrap();
        fake.stream = tokio::net::UnixStream::from_std(stream).unwrap();
        let left = Place {
            width: 1707,
            ..LEFT
        };
        fake.screen(A, left).await;
        std::future::pending::<()>().await;
    })
    .await;
    assert_eq!((layout, missing, fired), (None, Some(Missing::Lost), 1));
}

#[tokio::test]
async fn an_output_plugged_in_later_joins_the_layout() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (link, mut fake) = Fake::pair();
            let watched = watched(&local, &mut fake, link).await;
            let below = Place {
                x: 0,
                y: 1440,
                width: 1920,
                height: 1080,
            };
            timeout(BOUND, async {
                fake.offer(Offer::Output(C)).await;
                let callback = fake.until_sync().await;
                fake.screen(C, below).await;
                fake.answer(callback).await;
                let callback = fake.until_sync().await;
                fake.answer(callback).await;
                fired(&watched).await;
            })
            .await
            .unwrap();
            assert_eq!(watched.wayland.layout(), layout(&[LEFT, RIGHT, below]));
        })
        .await;
}

#[tokio::test]
async fn a_protocol_error_makes_the_layout_unknown_for_good() {
    let (layout, missing, fired) = ended_by(async |mut fake| {
        // wl_display.error about A's output: the object, a code, a message.
        let words = [fake.ids(A).output, 0]
            .into_iter()
            .chain(string("bad request"))
            .collect();
        let error = Message {
            object: DISPLAY,
            opcode: 0,
            words,
        };
        fake.send(vec![error]).await;
        // Kept open: the error alone ends it.
        std::future::pending::<()>().await;
    })
    .await;
    assert_eq!(
        (layout, missing, fired),
        (None, Some(Missing::ProtocolError), 1)
    );
}

#[test]
fn an_event_the_client_cannot_read_is_a_protocol_error() {
    // The pure-Rust backend the daemon is built with reports it so. (A
    // workspace build links libwayland, which logs it and goes quiet.)
    let malformed = DispatchError::BadMessage {
        sender_id: wayland_client::backend::ObjectId::null(),
        interface: "wl_output",
        opcode: 9,
    };
    assert_eq!(dispatch_error(malformed), Missing::ProtocolError);
    let closed = DispatchError::Backend(WaylandError::Io(ErrorKind::BrokenPipe.into()));
    assert_eq!(dispatch_error(closed), Missing::Lost);
}

#[test]
fn a_second_watch_and_a_handle_without_a_display_watch_nothing() {
    let none = Wayland::none(Missing::NoDisplay);
    assert_eq!(
        (none.layout(), none.missing(), none.compositor()),
        (None, Some(Missing::NoDisplay), None)
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let quiet = timeout(
            Duration::from_millis(50),
            none.watch(LayoutSignal::default()),
        )
        .await;
        assert!(quiet.is_err(), "a watch without a connection ended");
    });
}

#[tokio::test]
async fn a_display_that_does_not_answer_in_time_keeps_its_listener() {
    let (link, _fake) = Fake::pair();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
    let wayland = timeout(BOUND, Wayland::over(link, deadline)).await.unwrap();
    assert_eq!(wayland.missing(), Some(Missing::CannotConnect));
    assert_eq!(wayland.layout(), None);
    assert_eq!(
        wayland.compositor().map(NonZeroU32::get),
        Some(std::process::id())
    );
}
