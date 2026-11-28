//! The keeper with real processes: stand-ins for the window, started on a
//! socket pair.

use std::path::PathBuf;
use std::time::Duration;

use auto_clicker_core::controller::WindowLink;
use auto_clicker_core::message::{Failure, Problem, Quoted};
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::start;
use crate::window::keep_with;
use crate::window::tests::{capture, view};

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
    let open = keep_with(opener, program, start);
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
    let open = keep_with(opener, program, start);
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
    let open = keep_with(opener, program, start);
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
    let open = keep_with(opener, Ok(program), start);
    open.send(()).unwrap();
    let mut link = next_link(&mut links).await;
    let sent = timeout(Duration::from_secs(5), link.requests.recv())
        .await
        .expect("the request arrived");
    assert_eq!(sent.map(|request| request.id), Some(7));
}
