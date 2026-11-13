//! The controller's platforms: uinput devices or the desktop's portal for
//! output, and on either the hotkeys from the keyboards or from the
//! desktop's shortcuts, the screen layout from the Wayland display and the
//! pointer from KWin or Hyprland, and the clocks and directory recordings
//! need.

use std::future::pending;
use std::path::PathBuf;
use std::time::Duration;

use auto_clicker_backend_hyprland::{Found, Hyprland};
use auto_clicker_backend_kernel::{
    DeviceIdentity, InputSources, KernelBackend, local_time, monotonic_now, read_input,
};
use auto_clicker_backend_kwin::Kwin;
use auto_clicker_backend_portal::{PortalBackend, PortalOutput, read_shortcuts};
use auto_clicker_backend_wayland::Wayland;
use auto_clicker_core::controller::{
    HotkeyReader, InputLifetime, InputLoss, InputReader, LayoutSignal, LayoutWatch, OpenInput,
    Platform, ReadInput, Route,
};
use auto_clicker_core::input::Input;
use auto_clicker_core::message::{NoPointer, NoScreens};
use auto_clicker_core::recording::{Layout, LocalTime, RecordingFeed, recordings_dir};

/// Where hotkeys come from, on either output: the reader detection picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkeys {
    pub reader: InputReader,
}

impl Hotkeys {
    /// Starts the reader. Only the keyboard reader has
    /// devices of the program's `own` to leave out, a recording to feed and
    /// keys to capture; the desktop's is asked for the bindings at its first
    /// bind, and opens the desktop's settings when the controller asks.
    fn read(self, own: Vec<DeviceIdentity>, reading: ReadInput) -> HotkeyReader {
        match self.reader {
            InputReader::Devices => evdev_reader(own, reading),
            // The controller stops the reader by dropping it, which closes
            // the desktop's session.
            InputReader::Desktop => {
                let ReadInput {
                    bindings,
                    send,
                    commands,
                    ..
                } = reading;
                Box::pin(read_shortcuts(bindings, commands, move |event| {
                    send.send(event);
                }))
            }
        }
    }
}

/// What the desktop gives besides input: the layout from the Wayland
/// display, the pointer from KWin or Hyprland, whichever counts.
pub struct Desktop {
    pub wayland: Wayland,
    pub kwin: Kwin,
    pub hyprland: Hyprland,
}

impl Desktop {
    fn layout(&self) -> Result<Layout, NoScreens> {
        self.wayland.layout()
    }

    /// Whether KWin or Hyprland reports the pointer, or why neither does:
    /// KWin when it does, else a picked Hyprland's reason, else why KWin's
    /// script is unusable, else why Hyprland was refused, else nothing
    /// reports it. So a stale `HYPRLAND_INSTANCE_SIGNATURE` on Plasma never
    /// hides KWin's own reason.
    fn pointer(&self) -> Result<(), NoPointer> {
        pointer(self.kwin.pointer(), self.hyprland.found())
    }

    /// The one watch of the layout and the positions: the display's
    /// watcher fires `changed` when the layout or why it is unknown changes,
    /// KWin's fires it when the KWin counted changes and feeds `feed`'s
    /// recordings their positions, and each KWin of the display it sees
    /// come, the first included, has the display's watcher connect again
    /// (W01 brief, W5). Hyprland's feeds positions on Hyprland and fires it
    /// when why it gives none changes.
    fn watch(&self, feed: RecordingFeed, changed: LayoutSignal) -> LayoutWatch {
        let hyprland = self
            .hyprland
            .watch(self.wayland.clone(), feed.clone(), changed.clone());
        let pointer = changed.clone();
        joined(
            |restarted| self.kwin.watch(feed, restarted, pointer),
            |again| self.wayland.watch(changed, again),
            hyprland,
        )
    }
}

/// What [`Desktop::pointer`] says, from what KWin says and what was found
/// of Hyprland.
fn pointer(kwin: Result<(), NoPointer>, hyprland: Found) -> Result<(), NoPointer> {
    match (kwin, hyprland) {
        (Ok(()), _) => Ok(()),
        (_, Found::Picked(reason)) => reason,
        (Err(NoPointer::NoSource), Found::Refused(why)) => Err(why),
        (kwin, _) => kwin,
    }
}

pub struct Kernel {
    pub hotkeys: Hotkeys,
    pub desktop: Desktop,
}

impl Platform for Kernel {
    type Backend = KernelBackend;

    const LIFETIME: InputLifetime = InputLifetime::Kept;

    fn open_input(&mut self) -> OpenInput<KernelBackend> {
        let opened = KernelBackend::open();
        Box::pin(async move { opened })
    }

    fn event_clock(&self) -> Duration {
        monotonic_now()
    }

    fn local_time(&self) -> Option<LocalTime> {
        local_time()
    }

    fn recordings_dir(&self) -> Option<PathBuf> {
        recordings_dir()
    }

    fn read_input(
        &mut self,
        own: Option<&Input<KernelBackend>>,
        reading: ReadInput,
    ) -> HotkeyReader {
        // The reader keeps its own copy: the devices move into each run.
        let own: Vec<DeviceIdentity> = own
            .map(|input| input.backend().devices().to_vec())
            .unwrap_or_default();
        self.hotkeys.read(own, reading)
    }

    fn input_reader(&self) -> InputReader {
        self.hotkeys.reader
    }

    fn layout(&self) -> Result<Layout, NoScreens> {
        self.desktop.layout()
    }

    fn pointer(&self) -> Result<(), NoPointer> {
        self.desktop.pointer()
    }

    fn watch_layout(&mut self, feed: RecordingFeed, changed: LayoutSignal) -> LayoutWatch {
        self.desktop.watch(feed, changed)
    }
}

/// Output through the desktop's RemoteDesktop portal, one session per run.
pub struct Portal {
    pub output: PortalOutput,
    pub hotkeys: Hotkeys,
    pub desktop: Desktop,
}

impl Platform for Portal {
    type Backend = PortalBackend;

    const LIFETIME: InputLifetime = InputLifetime::PerRun;

    const OUTPUT: Route = Route::Portal;

    fn open_input(&mut self) -> OpenInput<PortalBackend> {
        Box::pin(self.output.open())
    }

    fn input_lost(&self, input: &Input<PortalBackend>) -> InputLoss {
        Box::pin(PortalOutput::lost(input))
    }

    fn event_clock(&self) -> Duration {
        monotonic_now()
    }

    fn local_time(&self) -> Option<LocalTime> {
        local_time()
    }

    fn recordings_dir(&self) -> Option<PathBuf> {
        recordings_dir()
    }

    fn input_reader(&self) -> InputReader {
        self.hotkeys.reader
    }

    /// libei's devices live inside the compositor and have no evdev node,
    /// so the reader has none of its own to ignore.
    fn read_input(
        &mut self,
        _own: Option<&Input<PortalBackend>>,
        reading: ReadInput,
    ) -> HotkeyReader {
        self.hotkeys.read(Vec::new(), reading)
    }

    fn layout(&self) -> Result<Layout, NoScreens> {
        self.desktop.layout()
    }

    fn pointer(&self) -> Result<(), NoPointer> {
        self.desktop.pointer()
    }

    fn watch_layout(&mut self, feed: RecordingFeed, changed: LayoutSignal) -> LayoutWatch {
        self.desktop.watch(feed, changed)
    }
}

/// KWin's watcher and the display's, polled together, the first one's word
/// of a restart reaching the second. KWin's is polled first, every time, so
/// the display hears of a restart in the same poll and fires the
/// controller's signal there, before a playback using positions takes its
/// next event, however much budget KWin's spent (the display's watcher
/// reads the word outside it). Hyprland's is polled last, so a layout the
/// display reads in a poll is published before Hyprland's watcher, in the
/// same poll, judges a position by it.
fn joined(
    kwin: impl FnOnce(LayoutSignal) -> LayoutWatch,
    display: impl FnOnce(LayoutSignal) -> LayoutWatch,
    hyprland: LayoutWatch,
) -> LayoutWatch {
    let restarted = LayoutSignal::default();
    let kwin = kwin(restarted.clone());
    let display = display(restarted);
    Box::pin(async move {
        tokio::join!(biased; kwin, display, hyprland);
    })
}

/// The evdev input reader, ignoring the devices `own`. It has nothing to
/// open in the desktop, so it takes no command.
fn evdev_reader(own: Vec<DeviceIdentity>, reading: ReadInput) -> HotkeyReader {
    let ReadInput {
        bindings,
        send,
        feed,
        capture,
        ..
    } = reading;
    // The controller stops the reader by dropping it, so it needs no
    // shutdown signal of its own.
    Box::pin(async move {
        let sources = InputSources {
            bindings,
            capture,
            own: &own,
            feed,
        };
        read_input(sources, |event| send.send(event), pending()).await;
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use std::cell::Cell;
    use std::rc::Rc;

    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use tokio::time::timeout;

    use super::*;

    fn no_display() -> Wayland {
        Wayland::none(NoScreens::NoDisplay)
    }

    fn hotkeys(reader: InputReader) -> Hotkeys {
        Hotkeys { reader }
    }

    fn desktop() -> Desktop {
        Desktop {
            wayland: no_display(),
            kwin: Kwin::none(),
            hyprland: Hyprland::none(),
        }
    }

    #[test]
    fn kwin_or_a_picked_hyprland_says_where_the_pointer_is_or_why_not() {
        use NoPointer::{HyprlandNoAnswer, HyprlandOtherDisplay, NoSource, Unusable};
        let refused = Found::Refused(HyprlandOtherDisplay);
        let cases = [
            (Ok(()), Found::Absent, Ok(())),
            (Ok(()), refused, Ok(())),
            (Ok(()), Found::Picked(Err(HyprlandNoAnswer)), Ok(())),
            (Err(NoSource), Found::Picked(Ok(())), Ok(())),
            (
                Err(NoSource),
                Found::Picked(Err(HyprlandNoAnswer)),
                Err(HyprlandNoAnswer),
            ),
            (
                Err(Unusable),
                Found::Picked(Err(HyprlandNoAnswer)),
                Err(HyprlandNoAnswer),
            ),
            // A stale signature on Plasma never hides KWin's own reason.
            (Err(Unusable), refused, Err(Unusable)),
            (Err(NoSource), refused, Err(HyprlandOtherDisplay)),
            (Err(NoSource), Found::Absent, Err(NoSource)),
            (Err(Unusable), Found::Absent, Err(Unusable)),
        ];
        for (kwin, hyprland, said) in cases {
            assert_eq!(pointer(kwin, hyprland), said, "{kwin:?} {hyprland:?}");
        }
    }

    #[test]
    fn the_picked_reader_and_what_is_missing_are_the_sources_on_either_output() {
        for reader in [InputReader::Devices, InputReader::Desktop] {
            let kernel = Kernel {
                hotkeys: hotkeys(reader),
                desktop: desktop(),
            };
            let portal = Portal {
                output: PortalOutput::new(None),
                hotkeys: hotkeys(reader),
                desktop: desktop(),
            };
            assert_eq!(kernel.input_reader(), reader);
            assert_eq!(portal.input_reader(), reader);
            // Without a display or KWin, each says why.
            let missing = (Err(NoScreens::NoDisplay), Err(NoPointer::Unusable));
            assert_eq!((kernel.layout(), kernel.pointer()), missing);
            assert_eq!((portal.layout(), portal.pointer()), missing);
        }
        assert_eq!(Kernel::OUTPUT, Route::Kernel);
        assert_eq!(Portal::OUTPUT, Route::Portal);
    }

    #[test]
    fn the_clocks_and_directory_are_the_backends_and_cores() {
        let kernel = Kernel {
            hotkeys: hotkeys(InputReader::Devices),
            desktop: desktop(),
        };
        let portal = Portal {
            output: PortalOutput::new(None),
            hotkeys: hotkeys(InputReader::Desktop),
            desktop: desktop(),
        };
        for (clock, local, dir) in [
            (
                kernel.event_clock(),
                kernel.local_time(),
                kernel.recordings_dir(),
            ),
            (
                portal.event_clock(),
                portal.local_time(),
                portal.recordings_dir(),
            ),
        ] {
            let gap = clock.abs_diff(monotonic_now());
            assert!(gap < Duration::from_secs(1), "{gap:?}");
            assert!(local.is_some());
            assert_eq!(dir, recordings_dir());
        }
    }

    #[tokio::test]
    async fn a_kwin_that_came_back_has_the_display_connect_again() {
        let connected_again = Rc::new(Cell::new(false));
        let seen = Rc::clone(&connected_again);
        let watch = joined(
            |restarted| {
                Box::pin(async move {
                    restarted.changed();
                    pending::<()>().await;
                })
            },
            |again| {
                Box::pin(async move {
                    again.fired().await;
                    seen.set(true);
                    pending::<()>().await;
                })
            },
            Box::pin(pending()),
        );
        let ran = timeout(Duration::from_secs(1), watch).await;
        assert!(ran.is_err(), "the watch ended");
        assert!(connected_again.get());
    }

    /// KWin's watcher, telling of a restart each time `staged` is set
    /// when it is polled.
    fn kwin_restarting(staged: Rc<Cell<bool>>) -> impl FnOnce(LayoutSignal) -> LayoutWatch {
        move |restarted| {
            Box::pin(std::future::poll_fn(move |_| {
                if staged.replace(false) {
                    restarted.changed();
                }
                Poll::Pending
            }))
        }
    }

    /// The display's watcher, firing `changed` on each restart it hears of.
    fn display_firing(changed: LayoutSignal) -> impl FnOnce(LayoutSignal) -> LayoutWatch {
        move |again| {
            Box::pin(async move {
                loop {
                    again.fired().await;
                    changed.changed();
                }
            })
        }
    }

    /// Whether `changed` fired, polled once.
    fn fired_now(changed: &LayoutSignal) -> bool {
        let mut cx = Context::from_waker(Waker::noop());
        std::pin::pin!(changed.fired()).poll(&mut cx).is_ready()
    }

    #[test]
    fn a_restart_reaches_the_controller_in_the_same_poll() {
        let staged = Rc::new(Cell::new(false));
        let changed = LayoutSignal::default();
        let mut watch = joined(
            kwin_restarting(Rc::clone(&staged)),
            display_firing(changed.clone()),
            Box::pin(pending()),
        );
        let mut cx = Context::from_waker(Waker::noop());
        // More polls than branches: each would have its turn to go first in
        // an unbiased join.
        for poll in 0..4 {
            staged.set(true);
            assert!(watch.as_mut().poll(&mut cx).is_pending());
            assert!(fired_now(&changed), "restart {poll} waited a poll");
        }
    }

    #[tokio::test]
    async fn a_watcher_that_ends_leaves_the_other_watched() {
        let polled = Rc::new(Cell::new(0));
        let seen = Rc::clone(&polled);
        let watch = joined(
            |_| Box::pin(async {}),
            |_| {
                Box::pin(std::future::poll_fn(move |_| {
                    seen.set(seen.get() + 1);
                    std::task::Poll::Pending
                }))
            },
            Box::pin(pending()),
        );
        let ran = timeout(Duration::from_millis(50), watch).await;
        assert!(ran.is_err(), "the watch ended with one watcher");
        assert!(polled.get() > 0);
    }

    #[test]
    fn hyprlands_watcher_is_polled_after_the_display_in_each_poll() {
        let order = Rc::new(std::cell::RefCell::new(Vec::new()));
        let polled = |name: &'static str| {
            let order = Rc::clone(&order);
            std::future::poll_fn(move |_| {
                order.borrow_mut().push(name);
                Poll::<()>::Pending
            })
        };
        let (kwin, display) = (polled("kwin"), polled("display"));
        let mut watch = joined(
            move |_| Box::pin(kwin),
            move |_| Box::pin(display),
            Box::pin(polled("hyprland")),
        );
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..3 {
            assert!(watch.as_mut().poll(&mut cx).is_pending());
        }
        let once = ["kwin", "display", "hyprland"];
        assert_eq!(*order.borrow(), once.repeat(3));
    }
}
