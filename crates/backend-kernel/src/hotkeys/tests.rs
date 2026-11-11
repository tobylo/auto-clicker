//! The reader against a temporary directory in place of `/dev/input`: the
//! watch, the status it reports and how it ends. Real keyboards are in the
//! hand-run device test.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use auto_clicker_core::hotkeys::{Bindings, HotkeyEvent, HotkeyStatus, Unavailable};
use auto_clicker_core::recording::RecordingSlot;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use super::access::look;
use super::node::Paths;
use super::{Reader, Setup, run};

/// Far beyond what any step here takes, and far below the wait it bounds,
/// which never ends by itself.
const BOUND: Duration = Duration::from_secs(5);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "auto-clicker-hotkeys-{}-{:016x}",
            std::process::id(),
            fastrand::u64(..)
        ));
        fs::create_dir(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A stand-in for `/dev/input` and `/sys/dev/char`.
struct Fixture {
    _root: TempDir,
    paths: Paths,
}

impl Fixture {
    fn new() -> Fixture {
        let root = TempDir::new();
        let paths = Paths {
            dev: root.0.join("input"),
            sys_char: root.0.join("char"),
        };
        for dir in [&paths.dev, &paths.sys_char] {
            fs::create_dir(dir).unwrap();
        }
        Fixture { _root: root, paths }
    }

    fn node(&self, name: &str) -> PathBuf {
        self.paths.dev.join(name)
    }
}

/// A node nobody may open. Made beside `path` and moved in, so it never
/// appears there readable.
fn unreadable(path: &Path) {
    let made = path.with_extension("new");
    fs::write(&made, b"").unwrap();
    fs::set_permissions(&made, fs::Permissions::from_mode(0o000)).unwrap();
    fs::rename(&made, path).unwrap();
}

/// Whether file permissions bind this process; not for root.
fn permissions_apply() -> bool {
    let dir = TempDir::new();
    let probe = dir.0.join("probe");
    unreadable(&probe);
    fs::File::open(&probe).is_err()
}

fn unavailable(why: Unavailable) -> HotkeyEvent {
    HotkeyEvent::Status(HotkeyStatus::Unavailable(why))
}

/// Runs the reader on `fixture` beside `drive`, which gets what the reader
/// sends and a trigger for shutdown. Both must finish within [`BOUND`].
async fn with_reader<F, Fut>(fixture: &Fixture, drive: F)
where
    F: FnOnce(mpsc::UnboundedReceiver<HotkeyEvent>, oneshot::Sender<()>) -> Fut,
    Fut: Future<Output = ()>,
{
    let (events, received) = mpsc::unbounded_channel();
    let (stop, stopped) = oneshot::channel::<()>();
    let send = move |event| {
        let _ = events.send(event);
    };
    let feed = RecordingSlot::new().feed();
    let setup = Setup {
        bindings: Bindings::default(),
        own: Vec::new(),
        feed,
    };
    let reader = Reader::new(fixture.paths.clone(), setup, send);
    let reader = run(reader, async move {
        let _ = stopped.await;
    });
    let both = async { tokio::join!(reader, drive(received, stop)) };
    timeout(BOUND, both)
        .await
        .expect("the reader did not end in time");
}

async fn next(received: &mut mpsc::UnboundedReceiver<HotkeyEvent>) -> Option<HotkeyEvent> {
    timeout(BOUND, received.recv()).await.ok().flatten()
}

#[tokio::test]
async fn no_keyboard_is_reported_and_shutdown_ends_the_wait() {
    let fixture = Fixture::new();
    let fx = &fixture;
    with_reader(fx, |mut received, stop| async move {
        assert_eq!(
            next(&mut received).await,
            Some(unavailable(Unavailable::NoKeyboard))
        );
        stop.send(()).unwrap();
    })
    .await;
}

/// What one look at `fixture`'s nodes finds.
fn looked(fixture: &Fixture) -> HotkeyStatus {
    look(fixture.paths.clone(), Bindings::default(), Vec::new())
}

#[tokio::test]
async fn one_look_finds_no_keyboard_or_no_permission_as_the_reader_would() {
    let fixture = Fixture::new();
    let no_keyboard = HotkeyStatus::Unavailable(Unavailable::NoKeyboard);
    assert_eq!(looked(&fixture), no_keyboard, "no node");
    // A file that is no device, and a node removed before it is looked at.
    fs::write(fixture.node("event1"), b"").unwrap();
    assert_eq!(looked(&fixture), no_keyboard, "no device");
    if !permissions_apply() {
        return;
    }
    // An unreadable name that is no event node counts for nothing.
    unreadable(&fixture.node("mice"));
    assert_eq!(looked(&fixture), no_keyboard, "not an event node");
    unreadable(&fixture.node("event2"));
    assert_eq!(
        looked(&fixture),
        HotkeyStatus::Unavailable(Unavailable::NoPermission)
    );
}

#[tokio::test]
async fn a_look_where_the_nodes_cannot_be_listed_says_so() {
    let fixture = Fixture::new();
    fs::remove_dir(&fixture.paths.dev).unwrap();
    assert!(
        matches!(
            looked(&fixture),
            HotkeyStatus::Unavailable(Unavailable::WatchFailed(_))
        ),
        "{:?}",
        looked(&fixture)
    );
}

#[tokio::test]
async fn shutdown_before_anything_happens_ends_at_once() {
    let fixture = Fixture::new();
    let fx = &fixture;
    with_reader(fx, |_, stop| async move {
        stop.send(()).unwrap();
    })
    .await;
}

#[tokio::test]
async fn nodes_that_are_no_keyboards_change_nothing() {
    if !permissions_apply() {
        return;
    }
    let fixture = Fixture::new();
    let fx = &fixture;
    with_reader(fx, |mut received, stop| async move {
        assert_eq!(
            next(&mut received).await,
            Some(unavailable(Unavailable::NoKeyboard))
        );
        // Neither an unreadable name that is no event node, nor a file that
        // is no device, changes the status; the denied node after them does.
        unreadable(&fx.node("mice"));
        fs::write(fx.node("event1"), b"").unwrap();
        unreadable(&fx.node("event2"));
        assert_eq!(
            next(&mut received).await,
            Some(unavailable(Unavailable::NoPermission))
        );
        stop.send(()).unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_node_that_cannot_be_opened_means_no_permission() {
    if !permissions_apply() {
        return;
    }
    let fixture = Fixture::new();
    let fx = &fixture;
    with_reader(fx, |mut received, stop| async move {
        assert_eq!(
            next(&mut received).await,
            Some(unavailable(Unavailable::NoKeyboard))
        );
        unreadable(&fx.node("event3"));
        assert_eq!(
            next(&mut received).await,
            Some(unavailable(Unavailable::NoPermission))
        );
        fs::remove_file(fx.node("event3")).unwrap();
        assert_eq!(
            next(&mut received).await,
            Some(unavailable(Unavailable::NoKeyboard))
        );
        stop.send(()).unwrap();
    })
    .await;
}

#[tokio::test]
async fn nodes_there_before_the_start_are_looked_at() {
    if !permissions_apply() {
        return;
    }
    let fixture = Fixture::new();
    unreadable(&fixture.node("event2"));
    let fx = &fixture;
    with_reader(fx, |mut received, stop| async move {
        assert_eq!(
            next(&mut received).await,
            Some(unavailable(Unavailable::NoPermission))
        );
        stop.send(()).unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_denied_node_is_retried_when_its_permissions_change() {
    if !permissions_apply() {
        return;
    }
    let fixture = Fixture::new();
    unreadable(&fixture.node("event4"));
    let fx = &fixture;
    with_reader(fx, |mut received, stop| async move {
        assert_eq!(
            next(&mut received).await,
            Some(unavailable(Unavailable::NoPermission))
        );
        // udev opens it up: now it is seen to be no keyboard at all.
        let mode = fs::Permissions::from_mode(0o644);
        fs::set_permissions(fx.node("event4"), mode).unwrap();
        assert_eq!(
            next(&mut received).await,
            Some(unavailable(Unavailable::NoKeyboard))
        );
        stop.send(()).unwrap();
    })
    .await;
}

#[tokio::test]
async fn losing_the_directory_ends_the_reader_and_says_why() {
    let fixture = Fixture::new();
    let fx = &fixture;
    with_reader(fx, |mut received, stop| async move {
        // Held, not sent: the reader must end by itself.
        let _stop = stop;
        assert_eq!(
            next(&mut received).await,
            Some(unavailable(Unavailable::NoKeyboard))
        );
        fs::remove_dir(&fx.paths.dev).unwrap();
        let Some(HotkeyEvent::Status(HotkeyStatus::Unavailable(Unavailable::WatchFailed(why)))) =
            next(&mut received).await
        else {
            panic!("no watch failure reported");
        };
        assert!(why.contains("went away"), "{why}");
    })
    .await;
}

#[tokio::test]
async fn a_missing_directory_is_reported() {
    let fixture = Fixture::new();
    fs::remove_dir(&fixture.paths.dev).unwrap();
    let fx = &fixture;
    with_reader(fx, |mut received, stop| async move {
        let _stop = stop;
        let event = next(&mut received).await;
        assert!(
            matches!(
                event,
                Some(HotkeyEvent::Status(HotkeyStatus::Unavailable(
                    Unavailable::WatchFailed(_)
                )))
            ),
            "{event:?}"
        );
    })
    .await;
}
