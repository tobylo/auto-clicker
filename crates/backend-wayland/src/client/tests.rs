//! The connection against a fake compositor on the other end of a socket
//! pair, speaking the wire protocol by hand: what is published when, and
//! how the connection ends.

use std::cell::Cell;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use auto_clicker_core::controller::LayoutSignal;
use auto_clicker_core::recording::{Layout, Output, Rect};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::LocalSet;
use tokio::time::timeout;

use super::*;
use crate::{Corner, Snapshot, Wayland};

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
    /// The manager at version 2: each xdg-output sends its own `done`.
    OldManager,
    Output(Name),
    /// A `wl_output` at an older version: 1 sends no `done`, 2 does.
    OldOutput(Name, u32),
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
const BELOW: Place = Place {
    x: 0,
    y: 1280,
    width: 1920,
    height: 1080,
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
        let fake = Fake::over(tokio::net::UnixStream::from_std(theirs).unwrap());
        (ours, fake)
    }

    /// The fake on its end of `stream`.
    fn over(stream: tokio::net::UnixStream) -> Fake {
        Fake {
            stream,
            outputs: Vec::new(),
            seen: Vec::new(),
        }
    }

    /// Whether the client closed its end: nothing more to read.
    async fn closed(&mut self) -> bool {
        let mut rest = Vec::new();
        self.stream.read_to_end(&mut rest).await.is_ok()
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
            Offer::OldManager => (MANAGER, "zxdg_output_manager_v1", 2),
            Offer::Output(name) => (name, "wl_output", 4),
            Offer::OldOutput(name, version) => (name, "wl_output", version),
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

/// A started connection's handle, watched on `local`, how often it fired,
/// and the signal of a new KWin; the watcher's task.
struct Watched {
    wayland: Wayland,
    fired: Rc<Cell<usize>>,
    again: LayoutSignal,
    watch: tokio::task::JoinHandle<()>,
}

/// Starts a connection on two outputs and watches it on `local`.
async fn watched(local: &LocalSet, fake: &mut Fake, link: UnixStream) -> Watched {
    let at = At {
        path: Some(PathBuf::from("/nowhere")),
        compositor: NonZeroU32::new(std::process::id()),
    };
    watched_at(local, fake, link, at).await
}

/// Where the display is: its socket, and the process found listening on it.
struct At {
    path: Option<PathBuf>,
    compositor: Option<NonZeroU32>,
}

/// The same, on the display `at`.
async fn watched_at(local: &LocalSet, fake: &mut Fake, link: UnixStream, at: At) -> Watched {
    let At { path, compositor } = at;
    let (started, ()) = timeout(BOUND, async {
        tokio::join!(Link::start(link), fake.start_with_two())
    })
    .await
    .unwrap();
    let (link, first) = started.unwrap();
    let wayland = Wayland::of(compositor, path, first, Ok(link));
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
    let again = LayoutSignal::default();
    let watch = local.spawn_local(wayland.watch(changed, again.clone()));
    Watched {
        wayland,
        fired,
        again,
        watch,
    }
}

/// Waits until the layout is unknown, for a reason.
async fn until_missing(wayland: &Wayland) {
    while wayland.layout().err().is_none() {
        settle().await;
    }
}

/// Waits until the layout is known.
async fn until_known(wayland: &Wayland) {
    while wayland.layout().is_err() {
        settle().await;
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
    assert_eq!(first.layout, layout(&[LEFT, RIGHT]));
}

#[tokio::test]
async fn with_xdg_output_2_the_layout_waits_for_the_xdg_outputs_done() {
    let (link, mut fake) = Fake::pair();
    let serve = async {
        let callback = fake.until_sync().await;
        fake.offer(Offer::OldManager).await;
        fake.offer(Offer::Output(A)).await;
        fake.answer(callback).await;
        let callback = fake.until_sync().await;
        let Ids { output, xdg } = fake.ids(A);
        let message = |object, opcode, words| Message {
            object,
            opcode,
            words,
        };
        // Its place and size, then wl_output's mode and done: the
        // xdg-output's update has not ended.
        fake.send(vec![
            message(xdg, 0, vec![0, 0]),
            message(xdg, 1, vec![2048, 1280]),
            message(output, 1, vec![1, 2048, 1280, 60_000]),
            message(output, 2, Vec::new()),
        ])
        .await;
        fake.answer(callback).await;
        let callback = fake.until_sync().await;
        fake.answer(callback).await;
        // Its xdg-output's done: now the layout is whole.
        fake.send(vec![message(xdg, 2, Vec::new())]).await;
        let callback = fake.until_sync().await;
        fake.answer(callback).await;
    };
    let (started, ()) = timeout(BOUND, async { tokio::join!(Link::start(link), serve) })
        .await
        .unwrap();
    assert_eq!(started.unwrap().1.layout, layout(&[LEFT]));
}

#[tokio::test]
async fn an_output_too_old_to_say_when_it_changed_leaves_the_layout_unknown() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (link, mut fake) = Fake::pair();
            let serve = async {
                let callback = fake.until_sync().await;
                fake.offer(Offer::Manager).await;
                fake.offer(Offer::Output(A)).await;
                fake.offer(Offer::OldOutput(B, 1)).await;
                fake.offer(Offer::OldOutput(C, 2)).await;
                fake.answer(callback).await;
                let callback = fake.until_sync().await;
                let bound: Vec<Name> = fake.outputs.iter().map(|(name, _)| *name).collect();
                assert_eq!(
                    bound,
                    [A, C],
                    "bound an output with no done, or not one with"
                );
                fake.screen(A, LEFT).await;
                fake.screen(C, BELOW).await;
                fake.answer(callback).await;
                let callback = fake.until_sync().await;
                fake.answer(callback).await;
            };
            let (started, ()) = timeout(BOUND, async { tokio::join!(Link::start(link), serve) })
                .await
                .unwrap();
            let (link, first) = started.unwrap();
            assert_eq!(first.layout, None, "a layout without the old output");
            // It goes: the layout is known.
            let wayland = Wayland::of(None, Some(PathBuf::from("/nowhere")), first, Ok(link));
            assert_eq!(wayland.layout(), Err(NoScreens::NotWhole));
            let changed = LayoutSignal::default();
            local.spawn_local(wayland.watch(changed.clone(), LayoutSignal::default()));
            timeout(BOUND, async {
                fake.send(vec![removed(B)]).await;
                let callback = fake.until_sync().await;
                fake.answer(callback).await;
                changed.fired().await;
            })
            .await
            .unwrap();
            assert_eq!(wayland.layout().ok(), layout(&[LEFT, BELOW]));
        })
        .await;
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
    assert_eq!(started.err(), Some(NoScreens::NoXdgOutput));
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
    assert_eq!(started.unwrap().1.layout, layout(&[LEFT]));

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
    assert_eq!(started.err(), Some(NoScreens::NoXdgOutput));
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
                watched.wayland.layout().ok(),
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
                let published = watched.wayland.layout().ok();
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
            assert_eq!(watched.wayland.layout().ok(), layout(&[left, right]));
            settle().await;
            assert_eq!(watched.fired.get(), 1);
            assert_eq!(watched.wayland.layout().err(), None);
        })
        .await;
}

#[tokio::test]
async fn an_update_behind_the_answer_is_waited_for() {
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
                fake.screen(A, left).await;
                let Callback(id) = fake.until_sync().await;
                // The answer and B's update arrive in one read.
                let mut answer_and_b = vec![
                    Message {
                        object: id,
                        opcode: 0,
                        words: vec![0],
                    },
                    Message {
                        object: DISPLAY,
                        opcode: 1,
                        words: vec![id],
                    },
                ];
                let Ids { output, xdg } = fake.ids(B);
                let message = |object, opcode, words| Message {
                    object,
                    opcode,
                    words,
                };
                answer_and_b.extend([
                    message(xdg, 0, vec![1707, 0]),
                    message(xdg, 1, vec![3440, 1440]),
                    message(output, 2, Vec::new()),
                ]);
                fake.send(answer_and_b).await;
                let again = fake.until_sync().await;
                settle().await;
                assert_eq!(
                    watched.wayland.layout().ok(),
                    layout(&[LEFT, RIGHT]),
                    "read A's update without B's behind the answer"
                );
                fake.answer(again).await;
                fired(&watched).await;
            })
            .await
            .unwrap();
            assert_eq!(watched.wayland.layout().ok(), layout(&[left, right]));
        })
        .await;
}

#[tokio::test]
async fn a_move_undone_before_the_answer_still_fires() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (link, mut fake) = Fake::pair();
            let watched = watched(&local, &mut fake, link).await;
            let away = Place { y: 1440, ..LEFT };
            timeout(BOUND, async {
                // A moves away; before the answer, it moves back.
                fake.screen(A, away).await;
                let callback = fake.until_sync().await;
                fake.screen(A, LEFT).await;
                fake.answer(callback).await;
                let callback = fake.until_sync().await;
                fake.answer(callback).await;
                fired(&watched).await;
            })
            .await
            .unwrap();
            settle().await;
            assert_eq!(watched.fired.get(), 1);
            assert_eq!(watched.wayland.layout().ok(), layout(&[LEFT, RIGHT]));
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
            assert_eq!(watched.wayland.layout().ok(), layout(&[LEFT]));
            // B's objects were released: its xdg-output destroyed, its
            // output (version 3 and later) released.
            let Ids { output, xdg } = fake.ids(B);
            assert!(fake.seen.contains(&(xdg, 0)), "B's xdg-output kept");
            assert!(fake.seen.contains(&(output, 0)), "B's output kept");
        })
        .await;
}

/// Starts with two outputs, watches, lets `end` end the connection from
/// the fake's side, and returns the handle and how often it fired. The
/// watcher runs on, waiting to be asked to connect again.
async fn ended_by(
    end: impl AsyncFnOnce(Fake) + 'static,
) -> (Option<Layout>, Option<NoScreens>, usize) {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (link, mut fake) = Fake::pair();
            let watched = watched(&local, &mut fake, link).await;
            local.spawn_local(async move { end(fake).await });
            timeout(BOUND, until_missing(&watched.wayland))
                .await
                .unwrap();
            settle().await;
            assert!(!watched.watch.is_finished(), "the watcher ended");
            let Watched { wayland, fired, .. } = watched;
            (wayland.layout().ok(), wayland.layout().err(), fired.get())
        })
        .await
}

#[tokio::test]
async fn a_lost_connection_makes_the_layout_unknown() {
    let (layout, missing, fired) = ended_by(async |fake| drop(fake)).await;
    assert_eq!((layout, missing, fired), (None, Some(NoScreens::Lost), 1));
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
    assert_eq!((layout, missing, fired), (None, Some(NoScreens::Lost), 1));
}

#[tokio::test]
async fn an_output_plugged_in_later_joins_the_layout() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (link, mut fake) = Fake::pair();
            let watched = watched(&local, &mut fake, link).await;
            let below = Place { y: 1440, ..BELOW };
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
            assert_eq!(watched.wayland.layout().ok(), layout(&[LEFT, RIGHT, below]));
        })
        .await;
}

#[tokio::test]
async fn a_protocol_error_makes_the_layout_unknown() {
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
        (None, Some(NoScreens::ProtocolError), 1)
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
    assert_eq!(dispatch_error(malformed), NoScreens::ProtocolError);
    let closed = DispatchError::Backend(WaylandError::Io(ErrorKind::BrokenPipe.into()));
    assert_eq!(dispatch_error(closed), NoScreens::Lost);
}

#[test]
fn a_handle_without_a_display_watches_nothing() {
    let none = Wayland::none(NoScreens::NoDisplay);
    assert_eq!(
        (none.layout(), none.compositor()),
        (Err(NoScreens::NoDisplay), None)
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let again = LayoutSignal::default();
        again.changed();
        let quiet = timeout(
            Duration::from_millis(50),
            none.watch(LayoutSignal::default(), again),
        )
        .await;
        assert!(quiet.is_err(), "a watch without a connection ended");
    });
    // A new KWin changed nothing: there is still no display.
    assert_eq!(none.layout(), Err(NoScreens::NoDisplay));
}

#[test]
fn a_new_reason_fires_while_the_layout_stays_unknown() {
    use std::future::Future;
    use std::task::{Context, Waker};

    let wayland = Wayland::none(NoScreens::Lost);
    let changed = LayoutSignal::default();
    let fired = |changed: &LayoutSignal| {
        let mut cx = Context::from_waker(Waker::noop());
        std::pin::pin!(changed.fired()).poll(&mut cx).is_ready()
    };
    wayland.0.end(NoScreens::Lost, &changed);
    assert!(!fired(&changed), "the same reason fired");
    assert_eq!(wayland.changes(), 0);
    wayland.0.end(NoScreens::CannotConnect, &changed);
    assert!(fired(&changed), "a new reason did not fire");
    assert_eq!(wayland.changes(), 1, "each fire counts as a change");
    assert_eq!(wayland.layout(), Err(NoScreens::CannotConnect));
    // A connection that gives no whole layout is a reason of its own.
    let half = Settled::default();
    wayland.0.publish(half, &changed);
    assert!(fired(&changed));
    assert_eq!(wayland.layout(), Err(NoScreens::NotWhole));
    // A whole layout comes with the corner it was moved from, as one value.
    let corner = Corner { x: -500, y: -300 };
    let whole = Settled {
        layout: layout(&[LEFT]),
        corner,
        moved: true,
    };
    wayland.0.publish(whole, &changed);
    let expected = Snapshot {
        layout: Ok(layout(&[LEFT]).unwrap()),
        corner,
        changes: 3,
    };
    assert_eq!(wayland.snapshot(), expected);
}

#[tokio::test]
async fn a_display_that_does_not_answer_in_time_keeps_its_listener() {
    let (link, _fake) = Fake::pair();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
    let path = PathBuf::from("/nowhere");
    let wayland = timeout(BOUND, Wayland::over(link, deadline, path))
        .await
        .unwrap();
    assert_eq!(wayland.layout().err(), Some(NoScreens::CannotConnect));
    assert_eq!(wayland.layout().ok(), None);
    assert_eq!(
        wayland.compositor().map(NonZeroU32::get),
        Some(std::process::id())
    );
}

/// A listening socket standing in for the display, at a path of its own.
struct Display {
    listener: tokio::net::UnixListener,
    path: PathBuf,
}

impl Display {
    fn new() -> Display {
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
    async fn accept(&self) -> Fake {
        let (stream, _) = self.listener.accept().await.unwrap();
        Fake::over(stream)
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A watched connection on two outputs to a display at its own path,
/// listened on by this process.
async fn watched_display(local: &LocalSet, display: &Display) -> (Watched, Fake) {
    let (link, mut fake) = Fake::pair();
    let at = At {
        path: Some(display.path.clone()),
        compositor: NonZeroU32::new(std::process::id()),
    };
    let watched = watched_at(local, &mut fake, link, at).await;
    (watched, fake)
}

#[tokio::test]
async fn a_new_kwin_connects_again_and_the_layout_comes_back() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (watched, fake) = watched_display(&local, &display).await;
            drop(fake);
            timeout(BOUND, until_missing(&watched.wayland))
                .await
                .unwrap();
            assert_eq!(watched.wayland.layout().ok(), None);
            watched.again.changed();
            let _fake = timeout(BOUND, async {
                let mut fake = display.accept().await;
                fake.start_with_two().await;
                until_known(&watched.wayland).await;
                fake
            })
            .await
            .unwrap();
            settle().await;
            assert_eq!(watched.wayland.layout().ok(), layout(&[LEFT, RIGHT]));
            assert_eq!(watched.wayland.layout().err(), None);
            assert_eq!(watched.fired.get(), 2, "unknown, then known again");
        })
        .await;
}

#[tokio::test]
async fn a_new_kwin_replaces_a_connection_still_open() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (watched, mut old) = watched_display(&local, &display).await;
            watched.again.changed();
            let _fake = timeout(BOUND, async {
                let mut fake = display.accept().await;
                assert!(old.closed().await, "the old connection stayed open");
                assert_eq!(watched.wayland.layout().ok(), None);
                fake.start_with_two().await;
                until_known(&watched.wayland).await;
                fake
            })
            .await
            .unwrap();
            settle().await;
            assert_eq!(watched.wayland.layout().ok(), layout(&[LEFT, RIGHT]));
        })
        .await;
}

#[tokio::test]
async fn a_newer_kwin_during_a_reconnect_starts_it_over() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (watched, fake) = watched_display(&local, &display).await;
            drop(fake);
            watched.again.changed();
            let _second = timeout(BOUND, async {
                // The first new connection is never answered; a newer KWin
                // comes meanwhile.
                let mut first = display.accept().await;
                watched.again.changed();
                let mut second = display.accept().await;
                assert!(first.closed().await, "the first try stayed open");
                second.start_with_two().await;
                until_known(&watched.wayland).await;
                second
            })
            .await
            .unwrap();
            settle().await;
            assert_eq!(watched.wayland.layout().err(), None);
        })
        .await;
}

#[tokio::test]
async fn a_display_another_process_listens_on_now_gives_no_layout() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (link, mut fake) = Fake::pair();
            // At start, the display's listener was another process.
            let at = At {
                path: Some(display.path.clone()),
                compositor: NonZeroU32::new(1),
            };
            let watched = watched_at(&local, &mut fake, link, at).await;
            drop(fake);
            watched.again.changed();
            timeout(BOUND, async {
                let mut fake = display.accept().await;
                assert!(fake.closed().await, "the connection stayed open");
                while watched.wayland.layout().err() != Some(NoScreens::CannotConnect) {
                    settle().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(watched.wayland.layout().ok(), None);
            assert!(!watched.watch.is_finished());
        })
        .await;
}

#[tokio::test]
async fn without_a_new_kwin_nothing_connects_again() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (watched, fake) = watched_display(&local, &display).await;
            drop(fake);
            timeout(BOUND, until_missing(&watched.wayland))
                .await
                .unwrap();
            let quiet = timeout(Duration::from_millis(200), display.accept()).await;
            assert!(quiet.is_err(), "connected again unasked");
            assert_eq!(watched.wayland.layout().err(), Some(NoScreens::Lost));
        })
        .await;
}

#[tokio::test]
async fn a_second_watch_watches_nothing() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (watched, _fake) = watched_display(&local, &display).await;
            // A signal of its own: the first watch never takes its fire.
            let again = LayoutSignal::default();
            local.spawn_local(
                watched
                    .wayland
                    .watch(LayoutSignal::default(), again.clone()),
            );
            again.changed();
            let quiet = timeout(Duration::from_millis(200), display.accept()).await;
            assert!(quiet.is_err(), "a second watch connected");
        })
        .await;
}

/// Connects again after a new KWin, and reads the layout back.
async fn comes_back(watched: &Watched, display: &Display) {
    watched.again.changed();
    let _fake = timeout(BOUND, async {
        let mut fake = display.accept().await;
        fake.start_with_two().await;
        until_known(&watched.wayland).await;
        fake
    })
    .await
    .unwrap();
    assert_eq!(watched.wayland.layout().ok(), layout(&[LEFT, RIGHT]));
    assert_eq!(watched.wayland.layout().err(), None);
}

#[tokio::test]
async fn a_display_that_failed_at_start_connects_at_the_next_kwin() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let wayland = Wayland::of(
                NonZeroU32::new(std::process::id()),
                Some(display.path.clone()),
                Settled::default(),
                Err(NoScreens::CannotConnect),
            );
            let again = LayoutSignal::default();
            let watch = local.spawn_local(wayland.watch(LayoutSignal::default(), again.clone()));
            let watched = Watched {
                wayland,
                fired: Rc::default(),
                again,
                watch,
            };
            comes_back(&watched, &display).await;
        })
        .await;
}

#[tokio::test]
async fn a_listener_unknown_at_start_takes_whoever_listens_now() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let display = Display::new();
            let (link, mut fake) = Fake::pair();
            let at = At {
                path: Some(display.path.clone()),
                compositor: None,
            };
            let watched = watched_at(&local, &mut fake, link, at).await;
            drop(fake);
            comes_back(&watched, &display).await;
        })
        .await;
}

#[tokio::test]
async fn a_new_kwin_makes_the_layout_unknown_in_its_poll_whatever_the_budget() {
    use std::future::{Future, poll_fn};
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    let (link, mut fake) = Fake::pair();
    let (started, ()) = timeout(BOUND, async {
        tokio::join!(Link::start(link), fake.start_with_two())
    })
    .await
    .unwrap();
    let (link, first) = started.unwrap();
    let wayland = Wayland::of(
        NonZeroU32::new(std::process::id()),
        Some(PathBuf::from("/nowhere")),
        first,
        Ok(link),
    );
    let (changed, again) = (LayoutSignal::default(), LayoutSignal::default());
    let mut watch = wayland.watch(changed.clone(), again.clone());
    // Polled here, in the test's own task, so the budget it spends counts.
    poll_fn(|cx| {
        assert!(watch.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    again.changed();
    poll_fn(|cx| {
        spend_budget(cx);
        assert!(watch.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let mut cx = Context::from_waker(Waker::noop());
    assert!(
        pin!(changed.fired()).poll(&mut cx).is_ready(),
        "the restart waited for a budget"
    );
    assert_eq!(
        (wayland.layout().ok(), wayland.layout().err()),
        (None, Some(NoScreens::Lost))
    );
}

/// Spends the task's whole budget.
fn spend_budget(cx: &mut std::task::Context<'_>) {
    use std::future::Future;

    use tokio::task::coop::consume_budget;

    while std::pin::pin!(consume_budget()).poll(cx).is_ready() {}
}
