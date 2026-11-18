//! A connection started against the fake and watched, and waits on what
//! it publishes.

use super::*;

/// A started connection's handle, watched on `local`, how often it fired,
/// and the signal of a new KWin; the watcher's task.
pub(super) struct Watched {
    pub(super) wayland: Wayland,
    pub(super) fired: Rc<Cell<usize>>,
    pub(super) again: LayoutSignal,
    pub(super) watch: tokio::task::JoinHandle<()>,
}

/// Starts a connection on two outputs and watches it on `local`.
pub(super) async fn watched(local: &LocalSet, fake: &mut Fake, link: UnixStream) -> Watched {
    let at = At {
        path: Some(PathBuf::from("/nowhere")),
        compositor: NonZeroU32::new(std::process::id()),
    };
    watched_at(local, fake, link, at).await
}

/// Where the display is: its socket, and the process found listening on it.
pub(super) struct At {
    pub(super) path: Option<PathBuf>,
    pub(super) compositor: Option<NonZeroU32>,
}

/// The same, on the display `at`.
pub(super) async fn watched_at(
    local: &LocalSet,
    fake: &mut Fake,
    link: UnixStream,
    at: At,
) -> Watched {
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
pub(super) async fn until_missing(wayland: &Wayland) {
    while wayland.layout().err().is_none() {
        settle().await;
    }
}

/// Waits until the layout is known.
pub(super) async fn until_known(wayland: &Wayland) {
    while wayland.layout().is_err() {
        settle().await;
    }
}

/// Lets every ready task run.
pub(super) async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// Waits until the signal fired.
pub(super) async fn fired(watched: &Watched) {
    while watched.fired.get() == 0 {
        settle().await;
    }
}

/// Sends the output `name` at `place` before answering the round trip the
/// last change asked for, then answers the next and waits for the signal.
pub(super) async fn settled_after(
    fake: &mut Fake,
    watched: &Watched,
    (name, place): (Name, Place),
) {
    let callback = fake.until_sync().await;
    fake.screen(name, place).await;
    fake.answer(callback).await;
    fake.round_trip().await;
    fired(watched).await;
}
