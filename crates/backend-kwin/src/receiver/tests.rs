use std::time::Duration;

use auto_clicker_core::recording::Point;
use tokio::net::UnixStream;
use tokio::time::timeout;
use zbus::connection::Builder;
use zbus::{Connection, Guid, Message};

use super::*;

const KWIN: &str = ":1.7";

fn at_42ms() -> Duration {
    Duration::from_millis(42)
}

fn token(seed: u64) -> Token {
    Token::new(&mut fastrand::Rng::with_seed(seed))
}

/// A receiver trusting `KWIN` with `tokens`, whose queue holds `room`.
fn receiver(tokens: Vec<Token>, room: usize) -> (Receiver, mpsc::Receiver<Report>) {
    let (queue, reports) = mpsc::channel(room);
    let trust = Trust {
        owner: Some(KWIN.to_owned()),
        tokens,
    };
    let receiver = Receiver {
        trust: Arc::new(Mutex::new(trust)),
        clock: at_42ms,
        queue,
        layout_dropped: Arc::new(Notify::new()),
    };
    (receiver, reports)
}

fn drain(reports: &mut mpsc::Receiver<Report>) -> Vec<Report> {
    std::iter::from_fn(|| reports.try_recv().ok()).collect()
}

fn position(token: &Token, x: u32) -> Report {
    Report {
        time: at_42ms(),
        token: token.clone(),
        body: Body::Position(Point { x, y: 5 }),
    }
}

#[test]
fn only_kwins_owner_with_a_current_token_is_heard() {
    let (current, stale) = (token(1), token(2));
    let (receiver, mut reports) = receiver(vec![current.clone()], QUEUE);
    receiver.position_from(Call {
        sender: None,
        token: current.as_str(),
        text: "1 5",
    });
    receiver.position_from(Call {
        sender: Some(":1.8"),
        token: current.as_str(),
        text: "2 5",
    });
    receiver.position_from(Call {
        sender: Some(KWIN),
        token: stale.as_str(),
        text: "3 5",
    });
    receiver.position_from(Call {
        sender: Some(KWIN),
        token: "not a token",
        text: "4 5",
    });
    receiver.layout_from(Call {
        sender: Some(":1.8"),
        token: current.as_str(),
        text: "0 0 10 10 1",
    });
    receiver.position_from(Call {
        sender: Some(KWIN),
        token: current.as_str(),
        text: "6 5",
    });
    assert_eq!(drain(&mut reports), [position(&current, 6)]);

    // Nobody owns org.kde.KWin: nothing is heard, whatever the token.
    with_trust(&receiver.trust, |trust| trust.owner = None);
    receiver.position_from(Call {
        sender: None,
        token: current.as_str(),
        text: "7 5",
    });
    receiver.position_from(Call {
        sender: Some(KWIN),
        token: current.as_str(),
        text: "8 5",
    });
    assert_eq!(drain(&mut reports), []);
}

#[test]
fn a_call_is_stamped_on_arrival_and_parsed_once() {
    let current = token(1);
    let (receiver, mut reports) = receiver(vec![current.clone()], QUEUE);
    receiver.position_from(Call {
        sender: Some(KWIN),
        token: current.as_str(),
        text: "-1 5",
    });
    receiver.layout_from(Call {
        sender: Some(KWIN),
        token: current.as_str(),
        text: "10 0 5 5 1",
    });
    receiver.layout_from(Call {
        sender: Some(KWIN),
        token: current.as_str(),
        text: "0 0 5 5 1",
    });
    let layout = report::layout("0 0 5 5 1");
    let layouts = [None, layout].map(|layout| Report {
        time: at_42ms(),
        token: current.clone(),
        body: Body::Layout(layout),
    });
    // No position for what is none; a layout that is none leaves the
    // layout unknown.
    assert_eq!(drain(&mut reports), layouts);
}

#[tokio::test]
async fn a_full_queue_drops_reports_and_says_so_for_a_layout() {
    let current = token(1);
    let (receiver, mut reports) = receiver(vec![current.clone()], 2);
    for x in 1..=3 {
        receiver.position_from(Call {
            sender: Some(KWIN),
            token: current.as_str(),
            text: &format!("{x} 5"),
        });
    }
    let quiet = timeout(
        Duration::from_millis(50),
        receiver.layout_dropped.notified(),
    )
    .await;
    assert!(
        quiet.is_err(),
        "a dropped position counted as a lost layout"
    );
    receiver.layout_from(Call {
        sender: Some(KWIN),
        token: current.as_str(),
        text: "0 0 5 5 1",
    });
    timeout(Duration::from_secs(5), receiver.layout_dropped.notified())
        .await
        .unwrap();
    // The queue kept what fitted, in order, and takes more once drained.
    assert_eq!(
        drain(&mut reports),
        [position(&current, 1), position(&current, 2)]
    );
    receiver.position_from(Call {
        sender: Some(KWIN),
        token: current.as_str(),
        text: "4 5",
    });
    assert_eq!(drain(&mut reports), [position(&current, 4)]);
}

/// A daemon's connection serving `receiver`, and a peer standing in for
/// the bus, which can say who sent each call.
async fn served(receiver: Receiver) -> (Connection, Connection) {
    let (ours, theirs) = UnixStream::pair().unwrap();
    let daemon = Builder::unix_stream(ours)
        .server(Guid::generate())
        .unwrap()
        .p2p()
        .serve_at(PATH, receiver)
        .unwrap()
        .build();
    let bus = Builder::unix_stream(theirs).p2p().build();
    tokio::try_join!(daemon, bus).unwrap()
}

/// A position call to the receiver, as `sender` sent it.
fn call(sender: &str, token: &Token, x: u32) -> Message {
    Message::method_call(PATH, "Position")
        .unwrap()
        .sender(sender)
        .unwrap()
        .interface("io.github.tobylo.AutoClicker.KWin")
        .unwrap()
        .build(&(token.as_str(), format!("{x} 5")))
        .unwrap()
}

/// On several threads, as zbus is free to use: a handler run as a task of
/// its own could then overtake the one before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn calls_are_handled_one_at_a_time_in_the_order_sent() {
    let current = token(1);
    let (receiver, mut reports) = receiver(vec![current.clone()], QUEUE);
    let (_daemon, bus) = served(receiver).await;
    for x in 0..300 {
        let sender = if x == 150 { ":1.8" } else { KWIN };
        bus.send(&call(sender, &current, x)).await.unwrap();
    }
    let mut seen = Vec::new();
    while seen.len() < 299 {
        let report = timeout(Duration::from_secs(10), reports.recv()).await;
        seen.push(report.unwrap().unwrap());
    }
    let expected: Vec<Report> = (0..300)
        .filter(|&x| x != 150)
        .map(|x| position(&current, x))
        .collect();
    assert_eq!(seen, expected);
}

#[tokio::test]
async fn a_layout_call_on_the_bus_arrives_as_a_layout() {
    let current = token(1);
    let (receiver, mut reports) = receiver(vec![current.clone()], QUEUE);
    let (_daemon, bus) = served(receiver).await;
    let call = Message::method_call(PATH, "Layout")
        .unwrap()
        .sender(KWIN)
        .unwrap()
        .interface("io.github.tobylo.AutoClicker.KWin")
        .unwrap()
        .build(&(current.as_str(), "0 0 5 5 1"))
        .unwrap();
    bus.send(&call).await.unwrap();
    let report = timeout(Duration::from_secs(10), reports.recv()).await;
    let expected = Report {
        time: at_42ms(),
        token: current,
        body: Body::Layout(report::layout("0 0 5 5 1")),
    };
    assert_eq!(report.unwrap(), Some(expected));
}
