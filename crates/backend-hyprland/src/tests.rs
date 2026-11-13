//! Finding Hyprland at start, against stand-ins for its socket in a
//! temporary runtime directory: this process listens on them.

use std::ffi::OsString;
use std::path::PathBuf;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

use super::*;

const SIGNATURE: &str = "11cbc6c_1763000000_123456789";

/// A runtime directory removed when dropped.
struct Runtime(PathBuf);

impl Runtime {
    fn new(name: &str) -> Runtime {
        let dir = std::env::temp_dir().join(format!(
            "auto-clicker-hyprland-find-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("hypr").join(SIGNATURE)).unwrap();
        Runtime(dir)
    }

    fn os(&self) -> Option<OsString> {
        Some(self.0.clone().into_os_string())
    }

    /// Hyprland's socket, answering each ask with `answer`.
    fn answering(&self, answer: &'static [u8]) {
        let path = self.0.join("hypr").join(SIGNATURE).join(".socket.sock");
        let listener = UnixListener::bind(path).unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0u8; 64];
                let _ = stream.read(&mut request).await;
                let _ = stream.write_all(answer).await;
            }
        });
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn this_process() -> Option<NonZeroU32> {
    NonZeroU32::new(std::process::id())
}

fn clock() -> Duration {
    Duration::ZERO
}

#[tokio::test]
async fn the_daemons_asks_go_only_to_the_process_found_at_start() {
    let runtime = Runtime::new("socket");
    runtime.answering(b"640, 400");
    let path = runtime.0.join("hypr").join(SIGNATURE).join(".socket.sock");
    let found = Socket {
        path: path.clone(),
        pid: std::process::id(),
    };
    assert!(found.ask().await.is_ok());
    let other = Socket {
        path,
        pid: std::process::id().wrapping_add(1),
    };
    assert_eq!(other.ask().await, Err(ask::Failure::OtherProcess));
}

async fn find(signature: Option<&str>, runtime: &Runtime, listener: Option<NonZeroU32>) -> Found {
    let signature = signature.map(OsString::from);
    Hyprland::find_at(signature, runtime.os(), listener, clock)
        .await
        .found()
}

#[tokio::test]
async fn without_a_signature_it_is_not_hyprland() {
    let runtime = Runtime::new("absent");
    assert_eq!(find(None, &runtime, this_process()).await, Found::Absent);
    assert_eq!(Hyprland::none().found(), Found::Absent);
}

#[tokio::test]
async fn hyprland_of_the_display_is_picked_and_says_whether_it_answers() {
    let runtime = Runtime::new("picked");
    runtime.answering(b"640, 400");
    let found = find(Some(SIGNATURE), &runtime, this_process()).await;
    assert_eq!(found, Found::Picked(Ok(())));

    let runtime = Runtime::new("unreadable");
    runtime.answering(b"640, 400\n");
    let found = find(Some(SIGNATURE), &runtime, this_process()).await;
    assert_eq!(found, Found::Picked(Err(NoPointer::HyprlandUnreadable)));
}

#[tokio::test]
async fn hyprland_the_daemon_cannot_tie_to_its_display_is_refused() {
    let refused = Found::Refused(NoPointer::HyprlandOtherDisplay);
    let runtime = Runtime::new("refused");
    runtime.answering(b"640, 400");
    // A signature that is not one path component.
    for bad in ["", "..", "a/b", "../11cbc6c_1763000000_123456789"] {
        assert_eq!(
            find(Some(bad), &runtime, this_process()).await,
            refused,
            "{bad:?}"
        );
    }
    // Nothing to check against.
    assert_eq!(find(Some(SIGNATURE), &runtime, None).await, refused);
    // Another display's listener: neither this process nor its parent.
    let other = NonZeroU32::new(u32::MAX - 1);
    assert_eq!(find(Some(SIGNATURE), &runtime, other).await, refused);
    // A stale signature: no socket there.
    let stale = Runtime::new("stale");
    assert_eq!(find(Some(SIGNATURE), &stale, this_process()).await, refused);
    // No runtime directory.
    let found = Hyprland::find_at(Some(SIGNATURE.into()), None, this_process(), clock).await;
    assert_eq!(found.found(), refused);
}

#[tokio::test]
async fn only_a_picked_hyprlands_first_watch_watches() {
    use std::task::{Context, Poll, Waker};

    let runtime = Runtime::new("watch");
    runtime.answering(b"640, 400");
    let picked =
        Hyprland::find_at(Some(SIGNATURE.into()), runtime.os(), this_process(), clock).await;
    let slot = auto_clicker_core::recording::RecordingSlot::new();
    let watch = |hyprland: &Hyprland| {
        let wayland = Wayland::none(auto_clicker_core::message::NoScreens::NoDisplay);
        hyprland.watch(wayland, slot.feed(), LayoutSignal::default())
    };
    let mut cx = Context::from_waker(Waker::noop());
    // A watcher that runs is pending too; a second watch is the one that
    // never runs, which the flag shows.
    let mut first = watch(&picked);
    assert!(matches!(first.as_mut().poll(&mut cx), Poll::Pending));
    assert!(picked.0.watched.get());
    let _second = watch(&picked);
    assert!(!Hyprland::none().0.watched.get());
    let none = Hyprland::none();
    let _nothing = watch(&none);
    assert!(!none.0.watched.get(), "watched without Hyprland");
}
