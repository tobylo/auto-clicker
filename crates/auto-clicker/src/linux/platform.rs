//! The controller's platforms: uinput devices or the desktop's portal for
//! output, and on either the hotkeys from the keyboards or from the
//! desktop's shortcuts, the screen layout from the Wayland display and the
//! pointer from KWin, Hyprland or the display's cursor sessions, the
//! on-screen display, and the clocks and directory recordings need.

use std::future::pending;
use std::path::PathBuf;
use std::time::Duration;

use auto_clicker_backend_hyprland::{Found, Hyprland};
use auto_clicker_backend_kernel::{
    DeviceIdentity, InputSources, KernelBackend, local_time, monotonic_now, read_input,
};
use auto_clicker_backend_kwin::Kwin;
use auto_clicker_backend_portal::{PortalBackend, PortalOutput, read_shortcuts};
use auto_clicker_backend_wayland::{Cursors, OnScreen, Wayland};
use auto_clicker_core::controller::{
    HotkeyReader, InputLifetime, InputLoss, InputReader, LayoutSignal, LayoutWatch, OpenInput,
    Platform, ReadInput, Route, View,
};
use auto_clicker_core::input::Input;
use auto_clicker_core::message::{NoOnScreen, NoPointer, NoScreens};
use auto_clicker_core::recording::{Layout, LocalTime, RecordingFeed, recordings_dir};
use tokio::sync::watch;

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
/// display, the pointer from KWin, Hyprland or the display's cursor
/// sessions, whichever counts, and the on-screen display.
pub struct Desktop {
    pub wayland: Wayland,
    pub kwin: Kwin,
    pub hyprland: Hyprland,
    pub cursors: Cursors,
    pub on_screen: OnScreen,
}

impl Desktop {
    fn layout(&self) -> Result<Layout, NoScreens> {
        self.wayland.layout()
    }

    /// Whether KWin, Hyprland or the display's cursor sessions report the
    /// pointer, or why none does: KWin when it does, else a found
    /// Hyprland's reason, else that a Hyprland socket did not answer when
    /// one could not be checked, else the cursor sessions' when the display
    /// offers them, else why KWin's script is unusable, else why Hyprland was
    /// refused, else nothing reports it. So a stale
    /// `HYPRLAND_INSTANCE_SIGNATURE` on Plasma never hides KWin's own
    /// reason, and cursor sessions never stand in for KWin's or Hyprland's
    /// route (W02).
    fn pointer(&self) -> Result<(), NoPointer> {
        pointer(
            self.kwin.pointer(),
            self.hyprland.found(),
            self.cursors.found(),
        )
    }

    /// The one watch of the layout and the positions: the display's
    /// watcher fires `changed` when the layout or why it is unknown changes,
    /// KWin's fires it when the KWin counted changes and feeds `feed`'s
    /// recordings their positions, and each KWin of the display it sees
    /// come, the first included, has the display's watcher connect again
    /// (W01 brief, W5). Hyprland's feeds positions on Hyprland and fires it
    /// when why it gives none changes.
    /// The cursor sessions' opens a connection for each recording that
    /// starts while neither KWin nor Hyprland reports the pointer, feeds it
    /// its positions, and fires it when why it gives none changes. The
    /// on-screen display's shows what `views` says should show.
    fn watch(
        &self,
        feed: RecordingFeed,
        views: watch::Receiver<View>,
        changed: LayoutSignal,
    ) -> LayoutWatch {
        let hyprland = self
            .hyprland
            .watch(self.wayland.clone(), feed.clone(), changed.clone());
        let (kwin, found) = (self.kwin.clone(), self.hyprland.clone());
        let open = Box::new(move || cursor_sessions_open(kwin.pointer(), found.found()));
        let cursors = self.cursors.watch(feed.clone(), open, changed.clone());
        let on_screen = self.on_screen.watch(views);
        let pointer = changed.clone();
        joined(
            |restarted| self.kwin.watch(feed, restarted, pointer),
            |again| self.wayland.watch(changed, again),
            [hyprland, cursors, on_screen],
        )
    }
}

/// Whether a recording that starts now opens cursor sessions: not when
/// KWin reports the pointer or Hyprland of this display was found, whose
/// own routes are measured, nor when a Hyprland socket could not be
/// checked (W02).
fn cursor_sessions_open(kwin: Result<(), NoPointer>, hyprland: Found) -> bool {
    kwin.is_err() && !matches!(hyprland, Found::Picked(_) | Found::Unsure)
}

/// What [`Desktop::pointer`] says, from what KWin says, what was found of
/// Hyprland, and the cursor sessions' reason when the display offers them.
fn pointer(
    kwin: Result<(), NoPointer>,
    hyprland: Found,
    cursors: Option<Result<(), NoPointer>>,
) -> Result<(), NoPointer> {
    match (kwin, hyprland, cursors) {
        (Ok(()), _, _) => Ok(()),
        (_, Found::Picked(reason), _) => reason,
        (_, Found::Unsure, _) => Err(NoPointer::HyprlandNoAnswer),
        (_, _, Some(reason)) => reason,
        (Err(NoPointer::NoSource), Found::Refused(why), None) => Err(why),
        (kwin, _, _) => kwin,
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

    fn on_screen(&self) -> Result<(), NoOnScreen> {
        self.desktop.on_screen.found()
    }

    fn watch_layout(
        &mut self,
        feed: RecordingFeed,
        views: watch::Receiver<View>,
        changed: LayoutSignal,
    ) -> LayoutWatch {
        self.desktop.watch(feed, views, changed)
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

    fn on_screen(&self) -> Result<(), NoOnScreen> {
        self.desktop.on_screen.found()
    }

    fn watch_layout(
        &mut self,
        feed: RecordingFeed,
        views: watch::Receiver<View>,
        changed: LayoutSignal,
    ) -> LayoutWatch {
        self.desktop.watch(feed, views, changed)
    }
}

/// KWin's watcher and the display's, polled together, the first one's word
/// of a restart reaching the second. KWin's is polled first, every time, so
/// the display hears of a restart in the same poll and fires the
/// controller's signal there, before a playback using positions takes its
/// next event, however much budget KWin's spent (the display's watcher
/// reads the word outside it). Hyprland's and the cursor sessions' are
/// polled last, in that order, so a layout the display reads in a poll is
/// published before Hyprland's watcher, in the same poll, judges a
/// position by it. The on-screen display's comes after them all: a draw
/// waits for the positions read in the same poll.
fn joined(
    kwin: impl FnOnce(LayoutSignal) -> LayoutWatch,
    display: impl FnOnce(LayoutSignal) -> LayoutWatch,
    [hyprland, cursors, on_screen]: [LayoutWatch; 3],
) -> LayoutWatch {
    let restarted = LayoutSignal::default();
    let kwin = kwin(restarted.clone());
    let display = display(restarted);
    Box::pin(async move {
        tokio::join!(biased; kwin, display, hyprland, cursors, on_screen);
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
        let wayland = no_display();
        Desktop {
            cursors: Cursors::of(&wayland, monotonic_now),
            on_screen: OnScreen::of(&wayland),
            wayland,
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
            assert_eq!(pointer(kwin, hyprland, None), said, "{kwin:?} {hyprland:?}");
        }
    }

    #[test]
    fn cursor_sessions_count_only_where_neither_kwin_nor_hyprland_reports() {
        use NoPointer::{
            DisplayNoPosition, HyprlandNoAnswer, HyprlandOtherDisplay, NoSource, Unusable,
        };
        let refused = Found::Refused(HyprlandOtherDisplay);
        let failed = Some(Err(DisplayNoPosition));
        let cases = [
            (Ok(()), Found::Absent, failed, Ok(())),
            (
                Err(NoSource),
                Found::Picked(Err(HyprlandNoAnswer)),
                Some(Ok(())),
                Err(HyprlandNoAnswer),
            ),
            (Err(NoSource), Found::Absent, Some(Ok(())), Ok(())),
            (Err(NoSource), Found::Absent, failed, Err(DisplayNoPosition)),
            // niri with a stale signature, or with no folder for KWin's
            // script: the cursor sessions report.
            (Err(NoSource), refused, Some(Ok(())), Ok(())),
            (Err(Unusable), Found::Absent, Some(Ok(())), Ok(())),
            // A Hyprland socket that could not be checked: no cursor
            // sessions stand in for it.
            (
                Err(NoSource),
                Found::Unsure,
                Some(Ok(())),
                Err(HyprlandNoAnswer),
            ),
        ];
        for (kwin, hyprland, cursors, said) in cases {
            let label = format!("{kwin:?} {hyprland:?} {cursors:?}");
            assert_eq!(pointer(kwin, hyprland, cursors), said, "{label}");
        }
        let opens = [
            (Ok(()), Found::Absent, false),
            (Err(NoSource), Found::Picked(Ok(())), false),
            (Err(Unusable), Found::Picked(Err(HyprlandNoAnswer)), false),
            (Err(NoSource), Found::Absent, true),
            (Err(Unusable), Found::Absent, true),
            (Err(NoSource), refused, true),
            (Err(NoSource), Found::Unsure, false),
        ];
        for (kwin, hyprland, open) in opens {
            assert_eq!(
                cursor_sessions_open(kwin, hyprland),
                open,
                "{kwin:?} {hyprland:?}"
            );
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
            assert_eq!(kernel.on_screen(), Err(NoOnScreen::NoDisplay));
            assert_eq!(portal.on_screen(), Err(NoOnScreen::NoDisplay));
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
            [
                Box::pin(pending()),
                Box::pin(pending()),
                Box::pin(pending()),
            ],
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
            [
                Box::pin(pending()),
                Box::pin(pending()),
                Box::pin(pending()),
            ],
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
            [
                Box::pin(pending()),
                Box::pin(pending()),
                Box::pin(pending()),
            ],
        );
        let ran = timeout(Duration::from_millis(50), watch).await;
        assert!(ran.is_err(), "the watch ended with one watcher");
        assert!(polled.get() > 0);
    }

    #[test]
    fn hyprlands_the_cursor_sessions_and_the_on_screen_watchers_are_polled_after_the_display() {
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
            [
                Box::pin(polled("hyprland")),
                Box::pin(polled("cursors")),
                Box::pin(polled("on screen")),
            ],
        );
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..3 {
            assert!(watch.as_mut().poll(&mut cx).is_pending());
        }
        let once = ["kwin", "display", "hyprland", "cursors", "on screen"];
        assert_eq!(*order.borrow(), once.repeat(3));
    }
}
