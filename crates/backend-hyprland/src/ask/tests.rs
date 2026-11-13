//! The ask against a stand-in for Hyprland's socket in a temporary
//! directory: this process listens, so its peer is this process.

use std::path::PathBuf;
use std::time::Instant;

use tokio::net::UnixListener;

use super::*;

/// A directory removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "auto-clicker-hyprland-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn socket(&self) -> PathBuf {
        self.0.join(".socket.sock")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ours(pid: u32) -> bool {
    pid == std::process::id()
}

/// A socket that reads one request and answers `answer`, then closes.
fn answering(dir: &TempDir, answer: &'static [u8]) -> tokio::task::JoinHandle<Vec<u8>> {
    let listener = UnixListener::bind(dir.socket()).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 64];
        let read = stream.read(&mut request).await.unwrap();
        stream.write_all(answer).await.unwrap();
        request[..read].to_vec()
    })
}

#[tokio::test]
async fn an_answer_is_read_to_its_end_and_parsed() {
    let dir = TempDir::new("answer");
    let served = answering(&dir, b"-1920, 300");
    let answer = ask(&dir.socket(), ours).await;
    assert_eq!(answer, Ok(Global { x: -1920, y: 300 }));
    assert_eq!(served.await.unwrap(), b"cursorpos");
}

#[tokio::test]
async fn an_answer_in_another_form_or_too_long_is_unreadable() {
    for (name, answer) in [("form", &b"640, 400\n"[..]), ("long", &[b'1'; 65][..])] {
        let dir = TempDir::new(name);
        let leaked: &'static [u8] = Box::leak(answer.to_vec().into_boxed_slice());
        let _served = answering(&dir, leaked);
        assert_eq!(
            ask(&dir.socket(), ours).await,
            Err(Failure::Unreadable),
            "{name}"
        );
    }
}

#[tokio::test]
async fn a_socket_of_another_process_is_asked_nothing() {
    let dir = TempDir::new("other");
    let listener = UnixListener::bind(dir.socket()).unwrap();
    let served = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).await.unwrap();
        request
    });
    let answer = ask(&dir.socket(), |pid| pid != std::process::id()).await;
    assert_eq!(answer, Err(Failure::OtherProcess));
    assert_eq!(served.await.unwrap(), b"", "something was written");
}

#[tokio::test]
async fn no_socket_is_no_answer() {
    let dir = TempDir::new("none");
    assert_eq!(ask(&dir.socket(), ours).await, Err(Failure::NoAnswer));
}

#[tokio::test]
async fn an_ask_that_is_never_answered_ends_at_its_deadline() {
    let dir = TempDir::new("silent");
    let listener = UnixListener::bind(dir.socket()).unwrap();
    let _held = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
        drop(stream);
    });
    let started = Instant::now();
    let asked = timeout(Duration::from_secs(5), ask(&dir.socket(), ours)).await;
    assert_eq!(asked, Ok(Err(Failure::NoAnswer)));
    assert!(started.elapsed() >= DEADLINE, "{:?}", started.elapsed());
    // Well under a second, the shortest wait anything else here has.
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn an_answer_that_came_while_the_daemon_was_busy_is_not_late() {
    // The daemon's one thread blocks past the deadline right after the ask
    // began, as it does opening the mice when a recording starts (78 ms
    // measured on the laptop); Hyprland is ready all along, on its own
    // thread here.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let dir = TempDir::new("busy");
    let listener = std::os::unix::net::UnixListener::bind(dir.socket()).unwrap();
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 9];
        stream.read_exact(&mut request).unwrap();
        stream.write_all(b"640, 400").unwrap();
    });
    let socket = dir.socket();
    let answer = runtime.block_on(async {
        let mut asking = Box::pin(ask(&socket, ours));
        // One poll starts the connect; the runtime has not seen it done.
        std::future::poll_fn(|cx| {
            assert!(asking.as_mut().poll(cx).is_pending(), "answered at once");
            std::task::Poll::Ready(())
        })
        .await;
        std::thread::sleep(DEADLINE + Duration::from_millis(30));
        asking.await
    });
    server.join().unwrap();
    assert_eq!(answer, Ok(Global { x: 640, y: 400 }));
}
