//! The connection against a fake compositor on the other end of a socket
//! pair, speaking the wire protocol by hand: what is published when, and
//! how the connection ends.

use std::cell::Cell;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use auto_clicker_core::controller::LayoutSignal;
use auto_clicker_core::recording::{Layout, Output};
use tokio::task::LocalSet;
use tokio::time::timeout;

use super::fake::*;
use super::*;
use crate::{Corner, Snapshot, Wayland};

mod watched;
use watched::*;

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
        fake.round_trip().await;
        // Its xdg-output's done: now the layout is whole.
        fake.send(vec![message(xdg, 2, Vec::new())]).await;
        fake.round_trip().await;
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
                fake.round_trip().await;
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
                fake.round_trip().await;
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
        fake.round_trip().await;
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
                fake.round_trip().await;
                // A removal after it shows the client handled the answer.
                fake.send(vec![removed(B)]).await;
                fake.round_trip().await;
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
                settled_after(&mut fake, &watched, (A, LEFT)).await;
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
async fn a_change_heard_is_unsettled_until_its_layout_is_published() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (link, mut fake) = Fake::pair();
            let watched = watched(&local, &mut fake, link).await;
            settle().await;
            let before = watched.wayland.changes();
            assert!(before.is_some());
            let away = Place { y: 1440, ..LEFT };
            timeout(BOUND, async {
                fake.screen(A, away).await;
                let callback = fake.until_sync().await;
                settle().await;
                assert_eq!(watched.wayland.changes(), None, "heard, yet settled");
                assert_eq!(watched.wayland.snapshot().changes, None);
                fake.answer(callback).await;
                fired(&watched).await;
            })
            .await
            .unwrap();
            let after = watched.wayland.changes();
            assert_eq!(after, before.map(|count| count + 1));
            assert_eq!(watched.wayland.snapshot().changes, after);
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
                fake.round_trip().await;
                // A removal after it shows the client handled the answer.
                fake.send(vec![removed(B)]).await;
                fake.round_trip().await;
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
                settled_after(&mut fake, &watched, (C, below)).await;
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
    assert_eq!(wayland.changes(), Some(0));
    wayland.0.end(NoScreens::CannotConnect, &changed);
    assert!(fired(&changed), "a new reason did not fire");
    assert_eq!(wayland.changes(), Some(1), "each fire counts as a change");
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
        changes: Some(3),
    };
    assert_eq!(wayland.snapshot(), expected);
}

mod again;
