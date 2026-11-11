//! The window's session, without a window: requests read from bytes and
//! updates written to a pipe, on tokio's paused clock.

use auto_clicker_core::config::{Config, Draft};
use auto_clicker_core::controller::Activity;
use auto_clicker_core::hotkeys::Capture;
use auto_clicker_core::window::{Ask, ShownHotkeys, WindowView};
use tokio::io::{AsyncBufReadExt, duplex};

use super::*;

fn capture(id: u64) -> Vec<u8> {
    line(&Request {
        id,
        ask: Ask::Capture(Capture::Hotkey),
    })
    .unwrap()
}

/// The requests read from `bytes` before reading stops.
async fn read_all(bytes: Vec<u8>) -> Vec<u64> {
    let (requests, mut received) = mpsc::channel(16);
    read_requests(BufReader::new(bytes.as_slice()), requests).await;
    let mut ids = Vec::new();
    while let Ok(request) = received.try_recv() {
        ids.push(request.id);
    }
    ids
}

#[tokio::test]
async fn requests_are_read_line_by_line_until_the_end() {
    let bytes = [capture(1), capture(2)].concat();
    assert_eq!(read_all(bytes).await, [1, 2]);
}

#[tokio::test]
async fn reading_stops_at_a_line_that_is_no_request() {
    let bytes = [capture(1), b"{\"id\":2}\n".to_vec(), capture(3)].concat();
    assert_eq!(read_all(bytes).await, [1]);
}

#[tokio::test]
async fn reading_stops_at_a_line_cut_short_or_too_long() {
    let mut cut = capture(2);
    cut.pop();
    assert_eq!(read_all([capture(1), cut].concat()).await, [1]);
    let long = [vec![b' '; MAX_LINE], capture(4)].concat();
    assert_eq!(read_all([capture(3), long].concat()).await, [3]);
}

#[tokio::test]
async fn reading_stops_when_the_controller_is_gone() {
    let (requests, received) = mpsc::channel(1);
    drop(received);
    let bytes = [capture(1), capture(2)].concat();
    timeout(
        Duration::from_secs(5),
        read_requests(BufReader::new(bytes.as_slice()), requests),
    )
    .await
    .expect("reading ended");
}

fn view() -> WindowView {
    let config = Config::default();
    let draft = Draft::from(&config);
    let shown = |binding| auto_clicker_core::controller::Hotkey::Bound(binding);
    WindowView {
        running: draft.input,
        settings: draft,
        activity: Activity::Idle,
        hotkeys: ShownHotkeys {
            toggle_clicking: shown(config.hotkeys.toggle_clicking),
            toggle_recording: shown(config.hotkeys.toggle_recording),
            toggle_playback: shown(config.hotkeys.toggle_playback),
        },
        no_recording: None,
        problems: Vec::new(),
        answer: None,
    }
}

#[tokio::test]
async fn views_and_raises_are_written_until_the_controller_ends_it() {
    let (ours, theirs) = duplex(MAX_LINE * 2);
    let (view_sender, mut views) = watch::channel(None);
    let (raise, mut raised) = mpsc::unbounded_channel();
    let writing = tokio::spawn(async move {
        let mut ours = ours;
        write_updates(&mut ours, &mut views, &mut raised).await;
    });
    let mut lines = BufReader::new(theirs).lines();
    view_sender.send_replace(Some(view()));
    let first = lines.next_line().await.unwrap().unwrap();
    assert_eq!(
        parse::<Update>(first.as_bytes()).unwrap(),
        Update::View(Box::new(view()))
    );
    raise.send(()).unwrap();
    let second = lines.next_line().await.unwrap().unwrap();
    assert_eq!(parse::<Update>(second.as_bytes()).unwrap(), Update::Raise);
    drop(view_sender);
    timeout(Duration::from_secs(5), writing)
        .await
        .expect("writing ended")
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_window_that_stops_reading_is_let_go_after_the_write_time() {
    let (mut ours, _theirs) = duplex(16);
    let (view_sender, mut views) = watch::channel(None);
    let (_raise, mut raised) = mpsc::unbounded_channel();
    view_sender.send_replace(Some(view()));
    let start = tokio::time::Instant::now();
    write_updates(&mut ours, &mut views, &mut raised).await;
    assert_eq!(start.elapsed(), WRITE_TIME);
}

#[test]
fn the_window_is_the_program_beside_the_daemon() {
    let program = program().unwrap();
    assert_eq!(program.file_name().unwrap(), PROGRAM);
    assert_eq!(program.parent(), std::env::current_exe().unwrap().parent());
}
