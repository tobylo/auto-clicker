//! The window's session, without a window: requests read from bytes and
//! updates written to a pipe, on tokio's paused clock.

use auto_clicker_core::config::{Config, Draft};
use auto_clicker_core::controller::Activity;
use auto_clicker_core::hotkeys::Capture;
use auto_clicker_core::message::{Failure, Problem, Quoted};
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
        routes: auto_clicker_core::controller::Routes::default(),
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
        write_updates(&mut ours, &mut views, &mut raised).await
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
    let stopped = timeout(Duration::from_secs(5), writing)
        .await
        .expect("writing ended")
        .unwrap();
    assert_eq!(stopped, Stopped::ControllerGone);
}

#[tokio::test(start_paused = true)]
async fn a_window_that_stops_reading_is_let_go_after_the_write_time() {
    let (mut ours, _theirs) = duplex(16);
    let (view_sender, mut views) = watch::channel(None);
    let (_raise, mut raised) = mpsc::unbounded_channel();
    view_sender.send_replace(Some(view()));
    let start = tokio::time::Instant::now();
    let stopped = write_updates(&mut ours, &mut views, &mut raised).await;
    assert_eq!(stopped, Stopped::WindowGone);
    assert_eq!(start.elapsed(), WRITE_TIME);
}

#[tokio::test]
async fn updates_say_whether_the_window_or_the_controller_went() {
    // The window closed its end: a write fails.
    let (mut ours, theirs) = duplex(16);
    drop(theirs);
    let (view_sender, mut views) = watch::channel(None);
    let (_raise, mut raised) = mpsc::unbounded_channel();
    view_sender.send_replace(Some(view()));
    let stopped = write_updates(&mut ours, &mut views, &mut raised).await;
    assert_eq!(stopped, Stopped::WindowGone);
    // The controller ended the session.
    let (mut ours, _theirs) = duplex(MAX_LINE);
    let (view_sender, mut views) = watch::channel(None);
    let (_raise, mut raised) = mpsc::unbounded_channel();
    drop(view_sender);
    let stopped = write_updates(&mut ours, &mut views, &mut raised).await;
    assert_eq!(stopped, Stopped::ControllerGone);
}

#[test]
fn the_window_is_the_program_beside_the_daemon() {
    let program = program().unwrap();
    assert_eq!(program.file_name().unwrap(), PROGRAM);
    assert_eq!(program.parent(), std::env::current_exe().unwrap().parent());
}

/// A directory of its own under the system's temporary one, removed when
/// dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "auto-clicker-window-test-{}-{:016x}",
            std::process::id(),
            fastrand::u64(..)
        ));
        std::fs::create_dir(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A stand-in for the window: notes its pid and whether it got `DISPLAY`
/// beside itself (in `window.pids`, so no path is written into the
/// script), and reads its standard input until the daemon closes it.
/// Writes `script` to `path` as a program, from a child shell: a file this
/// process held open for writing could be inherited by a child another test
/// forks at that moment, and running it would then fail with "Text file
/// busy" (ETXTBSY).
fn write_program(path: &std::path::Path, script: &str) {
    use std::io::Write;
    let mut shell = std::process::Command::new("sh")
        .args(["-c", "cat > \"$0\" && chmod 755 \"$0\""])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = shell.stdin.take().unwrap();
    input.write_all(script.as_bytes()).unwrap();
    drop(input);
    assert!(shell.wait().unwrap().success());
}

fn stand_in(dir: &std::path::Path) -> PathBuf {
    let program = dir.join("window");
    let script = "#!/bin/sh\necho \"$$ ${DISPLAY-none}\" >> \"$0.pids\"\nexec cat > /dev/null\n";
    write_program(&program, script);
    program
}

/// The pid the n-th window (from 0) noted, once it has; it got no
/// `DISPLAY`.
async fn pid(dir: &std::path::Path, n: usize) -> u32 {
    for _ in 0..100 {
        let noted = std::fs::read_to_string(dir.join("window.pids")).unwrap_or_default();
        if let Some(line) = noted.lines().nth(n) {
            let (pid, display) = line.split_once(' ').unwrap();
            assert_eq!(display, "none", "the window got DISPLAY");
            return pid.parse().unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("window {n} never started");
}

/// Waits until process `pid` is gone (or only a zombie), 5 s at most.
async fn gone(pid: u32) {
    for _ in 0..100 {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let state = stat
            .rsplit(')')
            .next()
            .and_then(|rest| rest.split_whitespace().next());
        if matches!(state, None | Some("Z")) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("window {pid} is still running");
}

/// The next window's link to the controller, 5 s at most.
async fn next_link(links: &mut mpsc::UnboundedReceiver<WindowLink>) -> WindowLink {
    timeout(Duration::from_secs(5), links.recv())
        .await
        .expect("a window opened")
        .unwrap()
}

#[tokio::test]
async fn one_window_runs_at_a_time_and_none_outlives_its_session() {
    let scratch = Scratch::new();
    let (opener, mut links) = mpsc::unbounded_channel();
    let program = Ok(stand_in(&scratch.0));
    let open = keep_with(Opening { opener, program });
    open.send(()).unwrap();
    let first = next_link(&mut links).await;
    // Chosen again: no second window.
    open.send(()).unwrap();
    assert!(
        timeout(Duration::from_millis(300), links.recv())
            .await
            .is_err()
    );
    // The controller ends the session: the window goes, and the next
    // choice opens a new one.
    let first_pid = pid(&scratch.0, 0).await;
    drop(first);
    gone(first_pid).await;
    open.send(()).unwrap();
    let _second = next_link(&mut links).await;
    let second_pid = pid(&scratch.0, 1).await;
    // The daemon quits: the keeper ends, and the window with it.
    drop(open);
    gone(second_pid).await;
}

/// A stand-in that never reads: writes to it block once the socket fills.
fn deaf_stand_in(dir: &std::path::Path) -> PathBuf {
    let program = dir.join("window");
    let script = "#!/bin/sh\necho \"$$ ${DISPLAY-none}\" >> \"$0.pids\"\nexec sleep 600\n";
    write_program(&program, script);
    program
}

#[tokio::test]
async fn the_daemon_quitting_ends_a_window_stuck_on_a_write_at_once() {
    use auto_clicker_core::window::{Answer, MAX_PROBLEMS, MAX_TEXT, Outcome};
    let scratch = Scratch::new();
    let (opener, mut links) = mpsc::unbounded_channel();
    let program = Ok(deaf_stand_in(&scratch.0));
    let open = keep_with(Opening { opener, program });
    open.send(()).unwrap();
    let link = next_link(&mut links).await;
    let window = pid(&scratch.0, 0).await;
    // Views until the socket is full and a write waits, as it may for
    // WRITE_TIME.
    let wide = "x".repeat(MAX_TEXT);
    for id in 0..64 {
        let mut shown = view();
        shown.problems = vec![Problem::Unlisted(Quoted::new(wide.clone())); MAX_PROBLEMS];
        shown.answer = Some(Answer {
            id,
            outcome: Outcome::Failed(Failure::Write(Quoted::new(wide.clone()))),
        });
        link.view.send_replace(Some(shown));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    drop(open);
    // Well under WRITE_TIME: the quit does not wait for the write.
    timeout(Duration::from_secs(2), gone(window))
        .await
        .expect("the window outlived the daemon's quit");
}

#[tokio::test]
async fn a_window_that_cannot_start_is_reported_and_tried_again() {
    let (opener, mut links) = mpsc::unbounded_channel();
    let program = Err("nowhere".to_owned());
    let open = keep_with(Opening { opener, program });
    for _ in 0..2 {
        open.send(()).unwrap();
        assert!(
            timeout(Duration::from_millis(200), links.recv())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn a_request_sent_just_before_the_window_exits_still_reaches_the_controller() {
    let scratch = Scratch::new();
    let program = scratch.0.join("window");
    let request = String::from_utf8(capture(7)).unwrap();
    // JSON has no single quote, so it sits in one safely.
    let script = format!("#!/bin/sh\nprintf '%s' '{request}' >&0\nexit 0\n");
    write_program(&program, &script);
    let (opener, mut links) = mpsc::unbounded_channel();
    let open = keep_with(Opening {
        opener,
        program: Ok(program),
    });
    open.send(()).unwrap();
    let mut link = next_link(&mut links).await;
    let sent = timeout(Duration::from_secs(5), link.requests.recv())
        .await
        .expect("the request arrived");
    assert_eq!(sent.map(|request| request.id), Some(7));
}
