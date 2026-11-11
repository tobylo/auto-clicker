//! The controller against a fake platform, on tokio's paused clock. Tests
//! drive it through messages and hotkey events only, and read what it does
//! from the view and from the calls its fake devices received.
//!
//! "At once" means the outcome is there before any virtual time passed:
//! `settle` only yields, so the paused clock cannot move while it runs, and a
//! controller that ignored a stop would still be waiting for its deadline.

use std::fmt;
use std::future::{Future, pending};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::task::{JoinHandle, LocalSet, yield_now};
use tokio::time::{Instant, sleep, timeout};

use super::*;
use crate::clicker::{ClickerSettings, Mode, Repeat};
use crate::hotkeys::Unavailable;
use crate::input::fake::{Call, FakeBackend, FakeLog};
use crate::input::{AbsolutePosition, Button, Key, PressState};
use crate::recording::{LocalTime, Recorder, RecordingFeed};
use crate::test_support::TempDir;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

const LEFT_DOWN: Call = Call::Button(Button::Left, PressState::Down);
const LEFT_UP: Call = Call::Button(Button::Left, PressState::Up);

// ---------------------------------------------------------------------------
// The fake platform

/// What the fake platform did, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    /// Devices opened: their id, or `None` when opening failed.
    Opened(Option<usize>),
    /// A reader started, ignoring the devices with this id.
    ReaderStarted(Option<usize>),
    /// The reader started n-th (from 0) was dropped.
    ReaderDropped(usize),
}

#[derive(Default)]
struct Record {
    steps: Vec<Step>,
    /// Every device call: the devices' id, the virtual time and the call.
    calls: Vec<(usize, Instant, Call)>,
    /// The controller's sleep reports, and the last one reported when each
    /// call was made: what a release came before.
    sleep_reports: Option<watch::Receiver<u64>>,
    reported_at_call: Vec<u64>,
    logs: Vec<FakeLog>,
    senders: Vec<HotkeySender>,
    failing_opens: usize,
    /// Readers send this and return, instead of running until dropped.
    reader_ends_with: Option<HotkeyEvent>,
    /// Readers are always ready, as under a flood of key events, and spend
    /// the task's budget on every poll without sending anything.
    busy_readers: bool,
    /// What a busy reader sends, once, when it gets to read after the
    /// devices were first used: a hotkey pressed during a run.
    pressed_during_a_run: Option<HotkeyEvent>,
    /// The feed each reader was given.
    feeds: Vec<RecordingFeed>,
    /// The bindings each reader was given.
    bindings: Vec<Bindings>,
    /// What each reader was started to capture, if anything.
    captures: Vec<Option<Capture>>,
    /// Each reader's commands from the controller.
    commands: Vec<ReaderCommands>,
    /// Where event-clock time 0 is.
    epoch: Option<Instant>,
    /// Where recordings are saved; none when `None`.
    recordings: Option<Arc<TempDir>>,
    /// The local time is unreadable.
    no_local_time: bool,
    /// The reader gets its hotkeys from the desktop.
    desktop_reader: bool,
    /// Opens wait until the test releases them through `open_gate`.
    hold_opens: bool,
    open_gate: Option<tokio::sync::oneshot::Sender<()>>,
    /// The screen layout the platform reports; unknown when `None`.
    layout: Option<Layout>,
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Record>>);

impl Shared {
    fn with<R>(&self, f: impl FnOnce(&mut Record) -> R) -> R {
        f(&mut self.0.lock().unwrap())
    }

    fn steps(&self) -> Vec<Step> {
        self.with(|r| r.steps.clone())
    }

    fn calls(&self) -> Vec<Call> {
        self.with(|r| r.calls.iter().map(|&(_, _, call)| call).collect())
    }

    /// The calls made to the devices opened n-th (0: those given at start).
    fn calls_to(&self, n: usize) -> Vec<Call> {
        self.with(|r| {
            r.calls
                .iter()
                .filter(|&&(id, _, _)| id == n)
                .map(|&(_, _, call)| call)
                .collect()
        })
    }

    /// When each call was made, relative to `start`.
    fn times_since(&self, start: Instant) -> Vec<(Duration, Call)> {
        self.with(|r| {
            r.calls
                .iter()
                .map(|&(_, at, call)| (at - start, call))
                .collect()
        })
    }

    /// Now on the fake event clock: virtual time since the daemon started.
    fn clock(&self) -> Duration {
        let epoch = self.with(|r| *r.epoch.get_or_insert_with(Instant::now));
        Instant::now() - epoch
    }

    /// Hands input to the recorder through the newest reader's feed.
    fn feed(&self, input: impl FnOnce(&mut Recorder)) {
        let feed = self.with(|r| r.feeds.last().unwrap().clone());
        feed.with(input);
    }

    fn log(&self, n: usize) -> FakeLog {
        self.with(|r| r.logs[n].clone())
    }

    /// Sends a hotkey event from the newest reader.
    fn hotkey(&self, event: HotkeyEvent) {
        self.with(|r| r.senders.last().unwrap().send(event));
    }

    fn devices(&self) -> Input<Devices> {
        Input::new(self.bare_devices())
    }

    /// New devices without the guard, for a test platform that wraps them.
    fn bare_devices(&self) -> Devices {
        let (inner, log) = FakeBackend::new();
        let id = self.with(|r| {
            r.logs.push(log);
            r.logs.len() - 1
        });
        Devices {
            id,
            inner,
            shared: self.clone(),
        }
    }
}

/// Fake devices: the fake backend, with an id, every call stamped.
struct Devices {
    id: usize,
    inner: FakeBackend,
    shared: Shared,
}

impl fmt::Debug for Devices {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Devices({})", self.id)
    }
}

impl Devices {
    fn note(&self, call: Call) {
        let (id, at) = (self.id, Instant::now());
        self.shared.with(|r| {
            r.calls.push((id, at, call));
            let reported = r
                .sleep_reports
                .as_ref()
                .map_or(0, |reports| *reports.borrow());
            r.reported_at_call.push(reported);
        });
    }
}

impl InputBackend for Devices {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        self.note(Call::Button(button, state));
        self.inner.button(button, state)
    }

    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError> {
        self.note(Call::Key(key, state));
        self.inner.key(key, state)
    }

    fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        self.note(Call::MoveRelative(dx, dy));
        self.inner.move_relative(dx, dy)
    }

    fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError> {
        self.note(Call::MoveAbsolute(position));
        self.inner.move_absolute(position)
    }

    fn scroll(&mut self, notches: i32) -> Result<(), InputError> {
        self.note(Call::Scroll(notches));
        self.inner.scroll(notches)
    }
}

/// Records when the reader it belongs to is dropped.
struct DropNote(Shared, usize);

impl Drop for DropNote {
    fn drop(&mut self) {
        let n = self.1;
        self.0.with(|r| r.steps.push(Step::ReaderDropped(n)));
    }
}

struct FakePlatform(Shared);

impl FakePlatform {
    fn open_now(&mut self) -> Result<Input<Devices>, InputError> {
        let fail = self.0.with(|r| {
            let fail = r.failing_opens > 0;
            r.failing_opens = r.failing_opens.saturating_sub(1);
            fail
        });
        if fail {
            self.0.with(|r| r.steps.push(Step::Opened(None)));
            return Err(InputError::CreateDevice(io::Error::other("injected")));
        }
        let input = self.0.devices();
        let id = input.backend().id;
        self.0.with(|r| r.steps.push(Step::Opened(Some(id))));
        Ok(input)
    }
}

impl Platform for FakePlatform {
    type Backend = Devices;

    const LIFETIME: InputLifetime = InputLifetime::Kept;

    /// Opens at once, as the kernel backend does, unless the test holds
    /// opens.
    fn open_input(&mut self) -> OpenInput<Devices> {
        let gate = self.0.with(|r| {
            r.hold_opens.then(|| {
                let (release, released) = tokio::sync::oneshot::channel();
                r.open_gate = Some(release);
                released
            })
        });
        let opened = self.open_now();
        Box::pin(async move {
            if let Some(released) = gate {
                let _ = released.await;
            }
            opened
        })
    }

    fn event_clock(&self) -> Duration {
        self.0.clock()
    }

    fn local_time(&self) -> Option<LocalTime> {
        let unreadable = self.0.with(|r| r.no_local_time);
        (!unreadable).then_some(LocalTime {
            year: 2026,
            month: 11,
            day: 9,
            hour: 18,
            minute: 4,
            second: 5,
            millis: 123,
            offset_seconds: 3600,
        })
    }

    fn recordings_dir(&self) -> Option<PathBuf> {
        self.0
            .with(|r| r.recordings.as_ref().map(|dir| dir.path().to_path_buf()))
    }

    fn layout(&self) -> Option<Layout> {
        self.0.with(|r| r.layout.clone())
    }

    fn input_reader(&self) -> InputReader {
        if self.0.with(|r| r.desktop_reader) {
            InputReader::Desktop
        } else {
            InputReader::Devices
        }
    }

    fn read_input(&mut self, own: Option<&Input<Devices>>, reading: ReadInput) -> HotkeyReader {
        let ReadInput {
            bindings,
            send,
            feed,
            capture,
            commands,
        } = reading;
        let own = own.map(|input| input.backend().id);
        let (n, ends_with, busy) = self.0.with(|r| {
            r.steps.push(Step::ReaderStarted(own));
            r.bindings.push(bindings);
            r.captures.push(capture);
            r.commands.push(commands);
            r.senders.push(send.clone());
            r.feeds.push(feed);
            (
                r.senders.len() - 1,
                r.reader_ends_with.clone(),
                r.busy_readers,
            )
        });
        let note = DropNote(self.0.clone(), n);
        let shared = self.0.clone();
        Box::pin(async move {
            let _note = note;
            match ends_with {
                Some(event) => send.send(event),
                None if busy => loop {
                    // Reading needs budget, as the real reader's does.
                    tokio::task::consume_budget().await;
                    let pressed = shared.with(|r| {
                        let running = !r.calls.is_empty();
                        r.pressed_during_a_run.take_if(|_| running)
                    });
                    pressed.into_iter().for_each(|event| send.send(event));
                },
                None => pending().await,
            }
        })
    }
}

// ---------------------------------------------------------------------------
// The harness

struct Daemon {
    inbox: mpsc::UnboundedSender<Message>,
    released_for_sleep: watch::Receiver<u64>,
    view: watch::Receiver<View>,
    shared: Shared,
    task: JoinHandle<()>,
}

fn config(mode: Mode) -> Config {
    Config {
        clicker: ClickerSettings {
            mode,
            ..ClickerSettings::default()
        },
        ..Config::default()
    }
}

/// Starts a controller on a local task, with an open session unless
/// `session_open` is false.
async fn daemon_with(config: Config, shared: Shared, session_open: bool) -> Daemon {
    daemon_prepared(config, shared, session_open, |_| {}).await
}

/// [`daemon_with`], with `prepare` given the controller before it runs.
async fn daemon_prepared(
    config: Config,
    shared: Shared,
    session_open: bool,
    prepare: impl FnOnce(&mut Controller<FakePlatform>),
) -> Daemon {
    shared.with(|r| {
        r.epoch = Some(Instant::now());
        r.recordings.get_or_insert_with(|| Arc::new(TempDir::new()));
    });
    let input = shared.devices();
    let platform = FakePlatform(shared.clone());
    let (mut controller, links) = Controller::new(platform, Some(input), config, None);
    prepare(&mut controller);
    let Links {
        send: inbox,
        view,
        released_for_sleep,
    } = links;
    shared.with(|r| r.sleep_reports = Some(released_for_sleep.clone()));
    let task = tokio::task::spawn_local(controller.run());
    let daemon = Daemon {
        inbox,
        released_for_sleep,
        view,
        shared,
        task,
    };
    if session_open {
        daemon.send(Message::Session(None)).await;
    }
    daemon.settle().await;
    daemon
}

async fn daemon(mode: Mode) -> Daemon {
    daemon_with(config(mode), Shared::default(), true).await
}

/// Runs a test body on a local set: the controller is not `Send`.
async fn local(body: impl Future<Output = ()>) {
    LocalSet::new().run_until(body).await;
}

impl Daemon {
    /// Lets the controller handle everything sent so far, without moving
    /// the clock.
    async fn settle(&self) {
        for _ in 0..20 {
            yield_now().await;
        }
    }

    /// Yields until `done` holds, at most 1000 times: bounded by polls, since
    /// a controller kept busy never lets the paused clock move.
    async fn settle_until(&self, done: impl Fn(&Daemon) -> bool) {
        for _ in 0..1000 {
            if done(self) {
                return;
            }
            yield_now().await;
        }
    }

    async fn send(&self, message: Message) {
        self.inbox.send(message).unwrap();
        self.settle().await;
    }

    async fn hotkey(&self, event: HotkeyEvent) {
        self.shared.hotkey(event);
        self.settle().await;
    }

    async fn press_hotkey(&self) {
        self.hotkey(HotkeyEvent::Fired(Action::ToggleClicking))
            .await;
    }

    fn view(&self) -> View {
        self.view.borrow().clone()
    }

    fn activity(&self) -> Activity {
        self.view.borrow().activity
    }

    fn notice(&self) -> String {
        self.view().notice.unwrap_or_default()
    }

    /// Asserts the controller is idle and that `calls` were all the calls.
    fn assert_idle_after(&self, calls: &[Call]) {
        assert_eq!(self.activity(), Activity::Idle);
        assert_eq!(self.shared.calls(), calls);
    }

    /// Starts clicking from the hotkey, with the modifiers coming up at once.
    async fn start_by_hotkey(&self) {
        self.press_hotkey().await;
        self.hotkey(HotkeyEvent::ModifiersReleased).await;
        assert_eq!(self.activity(), Activity::Clicking);
    }
}

// ---------------------------------------------------------------------------
// The tray countdown

#[tokio::test(start_paused = true)]
async fn a_tray_start_counts_down_and_clicks_on_the_deadline() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        let start = Instant::now();
        d.send(Message::Start).await;
        let mut shown = vec![d.activity()];
        for _ in 0..5 {
            sleep(secs(1)).await;
            d.settle().await;
            shown.push(d.activity());
        }
        use Activity::*;
        assert_eq!(
            shown,
            [
                StartingIn(5),
                StartingIn(4),
                StartingIn(3),
                StartingIn(2),
                StartingIn(1),
                Clicking
            ]
        );
        assert_eq!(d.shared.times_since(start), [(secs(5), LEFT_DOWN)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_zero_delay_starts_at_once() {
    local(async {
        let config = Config {
            tray_start_delay: Duration::ZERO,
            ..config(Mode::HoldButton)
        };
        let d = daemon_with(config, Shared::default(), true).await;
        let start = Instant::now();
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Clicking);
        assert_eq!(d.shared.times_since(start), [(Duration::ZERO, LEFT_DOWN)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn stop_and_the_hotkey_cancel_the_countdown_at_once() {
    for cancel in ["stop", "hotkey"] {
        local(async {
            let d = daemon(Mode::HoldButton).await;
            d.send(Message::Start).await;
            sleep(secs(2)).await;
            let before = Instant::now();
            match cancel {
                "stop" => d.send(Message::Stop).await,
                _ => d.press_hotkey().await,
            }
            assert_eq!(Instant::now(), before, "{cancel}");
            assert_eq!(d.activity(), Activity::Idle, "{cancel}");
            sleep(secs(10)).await;
            d.settle().await;
            d.assert_idle_after(&[]);
        })
        .await;
    }
}

// ---------------------------------------------------------------------------
// The hotkey and its modifiers

#[tokio::test(start_paused = true)]
async fn a_hotkey_start_clicks_when_the_modifiers_come_up() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        let start = Instant::now();
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
        sleep(secs(3)).await;
        d.settle().await;
        assert_eq!(d.shared.calls(), []);
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        assert_eq!(d.activity(), Activity::Clicking);
        assert_eq!(d.shared.times_since(start), [(secs(3), LEFT_DOWN)]);
    })
    .await;
}

/// Everything that ends a hotkey start waiting for the modifiers.
#[derive(Clone, Copy, Debug)]
enum Cancel {
    Stop,
    Hotkey,
    ReleaseLost,
    Unavailable,
    Locked,
    LockRequested,
    Quit,
}

impl Cancel {
    fn send(self, d: &Daemon) {
        let message = match self {
            Cancel::Hotkey => return d.shared.hotkey(HotkeyEvent::Fired(Action::ToggleClicking)),
            Cancel::ReleaseLost => return d.shared.hotkey(HotkeyEvent::ReleaseLost),
            Cancel::Unavailable => {
                let status = HotkeyStatus::Unavailable(Unavailable::NoKeyboard);
                return d.shared.hotkey(HotkeyEvent::Status(status));
            }
            Cancel::Stop => Message::Stop,
            Cancel::Locked => Message::Session(Some(Blocked::Locked)),
            Cancel::LockRequested => Message::LockRequested,
            Cancel::Quit => Message::Quit,
        };
        d.inbox.send(message).unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn every_cancel_ends_the_release_wait_at_once() {
    use Cancel::*;
    for cancel in [
        Stop,
        Hotkey,
        ReleaseLost,
        Unavailable,
        Locked,
        LockRequested,
        Quit,
    ] {
        local(async {
            let d = daemon(Mode::HoldButton).await;
            d.press_hotkey().await;
            sleep(secs(1)).await;
            let before = Instant::now();
            cancel.send(&d);
            d.settle().await;
            assert_eq!(Instant::now(), before, "{cancel:?}");
            if let Quit = cancel {
                assert!(d.task.is_finished());
                assert_eq!(d.shared.calls(), []);
                return;
            }
            assert_eq!(d.activity(), Activity::Idle, "{cancel:?}");
            // A release that still arrives starts nothing.
            d.hotkey(HotkeyEvent::ModifiersReleased).await;
            sleep(RELEASE_WAIT).await;
            d.settle().await;
            d.assert_idle_after(&[]);
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_release_wait_gives_up_after_its_deadline() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.press_hotkey().await;
        sleep(RELEASE_WAIT - Duration::from_millis(1)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
        sleep(Duration::from_millis(1)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Idle);
        assert!(d.notice().contains("held for 30 s"), "{}", d.notice());
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        d.assert_idle_after(&[]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_release_seen_only_after_the_deadline_starts_nothing() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.press_hotkey().await;
        // The release and the deadline are both ready when the controller
        // next runs; the release is looked at first.
        d.shared.hotkey(HotkeyEvent::ModifiersReleased);
        tokio::time::advance(RELEASE_WAIT).await;
        d.settle().await;
        d.assert_idle_after(&[]);
        assert!(d.notice().contains("held for 30 s"), "{}", d.notice());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_lost_release_says_to_press_again() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.press_hotkey().await;
        d.hotkey(HotkeyEvent::ReleaseLost).await;
        assert!(
            d.notice().contains("press Ctrl+Shift+F6 again"),
            "{}",
            d.notice()
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_release_without_a_pending_start_does_nothing() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.hotkey(HotkeyEvent::ModifiersReleased).await;
        d.hotkey(HotkeyEvent::Fired(Action::TogglePlayback)).await;
        d.assert_idle_after(&[]);
    })
    .await;
}

// ---------------------------------------------------------------------------
// Tray and hotkey together

#[tokio::test(start_paused = true)]
async fn tray_and_hotkey_alternate_consistently() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        // The hotkey cancels a tray countdown.
        d.send(Message::Start).await;
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::Idle);
        // A hotkey start, stopped from the tray.
        d.start_by_hotkey().await;
        d.send(Message::Stop).await;
        assert_eq!(d.activity(), Activity::Idle);
        // A tray start, stopped by the hotkey.
        d.send(Message::Start).await;
        sleep(secs(5)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Clicking);
        d.press_hotkey().await;
        assert_eq!(d.activity(), Activity::Idle);
        // Tray stops a hotkey start still waiting for the modifiers.
        d.press_hotkey().await;
        d.send(Message::Stop).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert_eq!(d.shared.calls(), [LEFT_DOWN, LEFT_UP, LEFT_DOWN, LEFT_UP]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_stale_tray_request_never_flips_the_mode() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.send(Message::Stop).await;
        assert_eq!(d.activity(), Activity::Idle);
        d.start_by_hotkey().await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Clicking);
        d.press_hotkey().await;
        // A Start during a hotkey start waiting for the modifiers.
        d.press_hotkey().await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
        d.press_hotkey().await;
        // A second Start a second into the countdown does not restart it.
        d.send(Message::Start).await;
        sleep(secs(1)).await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::StartingIn(4));
        sleep(secs(4)).await;
        d.settle().await;
        assert_eq!(d.shared.calls(), [LEFT_DOWN, LEFT_UP, LEFT_DOWN]);
    })
    .await;
}

// ---------------------------------------------------------------------------
// Shutting down

#[tokio::test(start_paused = true)]
async fn quit_while_holding_leaves_nothing_pressed() {
    for how in ["quit", "inbox closed"] {
        local(async {
            let d = daemon(Mode::HoldButton).await;
            d.start_by_hotkey().await;
            let before = Instant::now();
            let Daemon {
                inbox,
                task,
                shared,
                ..
            } = d;
            match how {
                "quit" => inbox.send(Message::Quit).unwrap(),
                _ => drop(inbox),
            }
            timeout(secs(1), task).await.unwrap().unwrap();
            assert_eq!(Instant::now(), before, "{how}");
            assert_eq!(shared.calls(), [LEFT_DOWN, LEFT_UP], "{how}");
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn quit_while_a_click_is_down_releases_it() {
    local(async {
        let d = daemon(Mode::Click).await;
        d.press_hotkey().await;
        d.shared.hotkey(HotkeyEvent::ModifiersReleased);
        // One poll: the run has pressed and waits in its hold.
        d.settle().await;
        assert_eq!(d.shared.calls(), [LEFT_DOWN]);
        let before = Instant::now();
        d.inbox.send(Message::Quit).unwrap();
        timeout(secs(1), d.task).await.unwrap().unwrap();
        // The hold inside the click is not waited out.
        assert_eq!(Instant::now(), before);
        assert_eq!(d.shared.calls(), [LEFT_DOWN, LEFT_UP]);
    })
    .await;
}

// ---------------------------------------------------------------------------
// The session

#[tokio::test(start_paused = true)]
async fn starts_are_refused_until_the_session_is_known() {
    local(async {
        let d = daemon_with(config(Mode::HoldButton), Shared::default(), false).await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Idle);
        d.press_hotkey().await;
        d.assert_idle_after(&[]);
        assert!(d.notice().contains("unknown"), "{}", d.notice());
        assert!(d.view().problems.iter().any(|p| p.contains("cannot start")));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_blocked_session_stops_clicking_and_refuses_starts() {
    let reasons = [
        Blocked::Locked,
        Blocked::Inactive,
        Blocked::Sleeping,
        Blocked::Unknown("lost".to_owned()),
    ];
    for reason in reasons {
        local(async {
            let d = daemon(Mode::HoldButton).await;
            d.start_by_hotkey().await;
            let before = Instant::now();
            d.send(Message::Session(Some(reason.clone()))).await;
            assert_eq!(Instant::now(), before, "{reason:?}");
            assert_eq!(d.activity(), Activity::Idle, "{reason:?}");
            assert_eq!(d.notice(), format!("Stopped: {reason}"));
            d.send(Message::Start).await;
            assert_eq!(d.activity(), Activity::Idle, "{reason:?}");
            d.press_hotkey().await;
            d.assert_idle_after(&[LEFT_DOWN, LEFT_UP]);
            assert_eq!(d.notice(), format!("Not started: {reason}"));
            // Open again, it starts.
            d.send(Message::Session(None)).await;
            d.start_by_hotkey().await;
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_blocked_session_cancels_the_countdown() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.send(Message::Start).await;
        d.send(Message::Session(Some(Blocked::Sleeping))).await;
        sleep(secs(10)).await;
        d.settle().await;
        d.assert_idle_after(&[]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_sleep_notice_is_reported_only_once_nothing_is_held() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.start_by_hotkey().await;
        let before = Instant::now();
        d.send(Message::Sleep(7)).await;
        assert_eq!(Instant::now(), before);
        assert_eq!(*d.released_for_sleep.borrow(), 7);
        // The release was written before the report.
        assert_eq!(d.shared.calls(), [LEFT_DOWN, LEFT_UP]);
        assert_eq!(d.shared.with(|r| r.reported_at_call.clone()), [0, 0]);
        assert_eq!(d.activity(), Activity::Idle);
        // Starts are refused until the session says otherwise.
        d.send(Message::Start).await;
        d.press_hotkey().await;
        d.assert_idle_after(&[LEFT_DOWN, LEFT_UP]);
        assert_eq!(d.notice(), format!("Not started: {}", Blocked::Sleeping));
        d.send(Message::Session(None)).await;
        d.start_by_hotkey().await;
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_sleep_notice_while_idle_or_counting_down_is_reported_at_once() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.send(Message::Sleep(1)).await;
        assert_eq!(*d.released_for_sleep.borrow(), 1);
        d.send(Message::Session(None)).await;
        d.send(Message::Start).await;
        d.send(Message::Sleep(2)).await;
        assert_eq!(*d.released_for_sleep.borrow(), 2);
        sleep(secs(10)).await;
        d.settle().await;
        d.assert_idle_after(&[]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_lock_request_stops_clicking_but_does_not_block() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.start_by_hotkey().await;
        d.send(Message::LockRequested).await;
        assert_eq!(d.activity(), Activity::Idle);
        d.start_by_hotkey().await;
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_block_while_idle_says_nothing_stopped_and_stop_keeps_the_notice() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.send(Message::Session(Some(Blocked::Locked))).await;
        d.send(Message::LockRequested).await;
        assert_eq!(d.view().notice, None);
        d.send(Message::Session(None)).await;
        // A run that ended with a notice, then a plain Stop while idle and
        // during the next run: the notice stays until a run starts.
        d.press_hotkey().await;
        d.hotkey(HotkeyEvent::ReleaseLost).await;
        let lost = d.notice();
        d.send(Message::Stop).await;
        assert_eq!(d.notice(), lost);
        d.send(Message::Start).await;
        d.send(Message::Stop).await;
        assert_eq!(d.notice(), lost);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn hotkey_trouble_only_cancels_a_start_waiting_for_the_modifiers() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.start_by_hotkey().await;
        d.hotkey(HotkeyEvent::ReleaseLost).await;
        let unavailable = HotkeyStatus::Unavailable(Unavailable::NoKeyboard);
        d.hotkey(HotkeyEvent::Status(unavailable)).await;
        assert_eq!(d.activity(), Activity::Clicking);
        assert_eq!(d.shared.calls(), [LEFT_DOWN]);
        d.send(Message::Stop).await;
        d.press_hotkey().await;
        d.hotkey(HotkeyEvent::Status(HotkeyStatus::Available)).await;
        assert_eq!(d.activity(), Activity::WaitingForRelease);
    })
    .await;
}

// ---------------------------------------------------------------------------
// Losing the devices

#[tokio::test(start_paused = true)]
async fn lost_devices_are_opened_again_after_the_old_reader_is_dropped() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.start_by_hotkey().await;
        d.shared.log(0).fail_once(LEFT_UP);
        d.send(Message::Stop).await;
        assert_eq!(d.activity(), Activity::Idle);
        assert!(d.notice().contains("could not write"), "{}", d.notice());
        assert_eq!(
            d.shared.steps(),
            [
                Step::ReaderStarted(Some(0)),
                Step::ReaderDropped(0),
                Step::Opened(Some(1)),
                Step::ReaderStarted(Some(1)),
            ]
        );
        // The next run uses the new devices.
        d.start_by_hotkey().await;
        assert_eq!(d.shared.calls_to(1), [LEFT_DOWN]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn kept_devices_stay_open_between_runs() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.start_by_hotkey().await;
        d.send(Message::Stop).await;
        d.start_by_hotkey().await;
        assert_eq!(d.shared.calls_to(0), [LEFT_DOWN, LEFT_UP, LEFT_DOWN]);
        assert_eq!(d.shared.steps(), [Step::ReaderStarted(Some(0))]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn when_opening_again_fails_the_next_start_tries_again() {
    local(async {
        let shared = Shared::default();
        shared.with(|r| r.failing_opens = 1);
        let d = daemon_with(config(Mode::HoldButton), shared, true).await;
        d.start_by_hotkey().await;
        d.shared.log(0).fail_once(LEFT_UP);
        d.send(Message::Stop).await;
        let problems = d.view().problems;
        assert!(
            problems.iter().any(|p| p.contains("injected")),
            "{problems:?}"
        );
        d.send(Message::Start).await;
        sleep(secs(5)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Clicking);
        assert!(d.view().problems.is_empty());
        assert_eq!(
            d.shared.steps(),
            [
                Step::ReaderStarted(Some(0)),
                Step::ReaderDropped(0),
                Step::Opened(None),
                Step::ReaderStarted(None),
                Step::ReaderDropped(1),
                Step::Opened(Some(1)),
                Step::ReaderStarted(Some(1)),
            ]
        );
        assert_eq!(d.shared.calls_to(1), [LEFT_DOWN]);
    })
    .await;
}

// ---------------------------------------------------------------------------
// Runs that end by themselves, readers that end, and what the view shows

#[tokio::test(start_paused = true)]
async fn a_counted_run_finishes_by_itself() {
    local(async {
        let mut config = config(Mode::Click);
        config.clicker.repeat = Repeat::Times(std::num::NonZeroU32::new(2).unwrap());
        let d = daemon_with(config, Shared::default(), true).await;
        d.start_by_hotkey().await;
        sleep(secs(1)).await;
        d.settle().await;
        d.assert_idle_after(&[LEFT_DOWN, LEFT_UP, LEFT_DOWN, LEFT_UP]);
        assert_eq!(d.notice(), "Finished every action");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_busy_reader_starves_neither_deadlines_nor_stops() {
    local(async {
        let shared = Shared::default();
        shared.with(|r| r.busy_readers = true);
        let d = daemon_with(config(Mode::HoldButton), shared, true).await;
        // The release wait still runs out.
        d.press_hotkey().await;
        tokio::time::advance(RELEASE_WAIT).await;
        d.settle_until(|d| d.activity() == Activity::Idle).await;
        assert_eq!(d.activity(), Activity::Idle);
        // The countdown still ends.
        d.send(Message::Start).await;
        tokio::time::advance(secs(5)).await;
        d.settle_until(|d| d.shared.calls() == [LEFT_DOWN]).await;
        assert_eq!(d.activity(), Activity::Clicking);
        assert_eq!(d.shared.calls(), [LEFT_DOWN]);
        // The hotkey still stops it: what the reader sent comes first.
        d.shared.hotkey(HotkeyEvent::Fired(Action::ToggleClicking));
        d.settle_until(|d| d.activity() == Activity::Idle).await;
        d.assert_idle_after(&[LEFT_DOWN, LEFT_UP]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_lock_queued_with_a_release_or_a_due_countdown_starts_nothing() {
    for by_tray in [false, true] {
        local(async {
            let d = daemon(Mode::HoldButton).await;
            if by_tray {
                d.send(Message::Start).await;
            } else {
                d.press_hotkey().await;
                d.shared.hotkey(HotkeyEvent::ModifiersReleased);
            }
            d.inbox
                .send(Message::Session(Some(Blocked::Locked)))
                .unwrap();
            tokio::time::advance(secs(5)).await;
            d.settle().await;
            d.assert_idle_after(&[]);
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_flood_of_tray_requests_cannot_keep_a_stopped_run_holding() {
    local(async {
        let d = daemon(Mode::HoldButton).await;
        d.start_by_hotkey().await;
        // More requests than `settle`'s polls can drain at one budget each,
        // all queued at once.
        d.inbox.send(Message::Stop).unwrap();
        for _ in 0..10_000 {
            d.inbox.send(Message::Start).unwrap();
        }
        d.settle().await;
        assert!(d.shared.calls().contains(&LEFT_UP));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_reader_that_ends_leaves_the_controller_working() {
    local(async {
        let shared = Shared::default();
        let failed = Unavailable::WatchFailed("gone".to_owned());
        let status = HotkeyEvent::Status(HotkeyStatus::Unavailable(failed.clone()));
        shared.with(|r| r.reader_ends_with = Some(status));
        let d = daemon_with(config(Mode::HoldButton), shared, true).await;
        assert!(d.view().problems.contains(&failed.to_string()));
        assert_eq!(
            d.shared.steps(),
            [Step::ReaderStarted(Some(0)), Step::ReaderDropped(0)]
        );
        d.send(Message::Start).await;
        sleep(secs(5)).await;
        d.settle().await;
        assert_eq!(d.activity(), Activity::Clicking);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn the_view_shows_the_config_error_and_hotkey_problems() {
    local(async {
        let shared = Shared::default();
        let input = shared.devices();
        let error = Some("config.toml:3: bad".to_owned());
        let platform = FakePlatform(shared.clone());
        let (controller, links) = Controller::new(platform, Some(input), Config::default(), error);
        let view = links.view;
        assert_eq!(view.borrow().title(), "Auto clicker: idle");
        let task = tokio::task::spawn_local(controller.run());
        yield_now().await;
        shared.hotkey(HotkeyEvent::Status(HotkeyStatus::Unavailable(
            Unavailable::NoPermission,
        )));
        for _ in 0..5 {
            yield_now().await;
        }
        let problems = view.borrow().problems.clone();
        assert_eq!(problems[0], "config.toml:3: bad");
        assert_eq!(problems[1], Unavailable::NoPermission.to_string());
        task.abort();
    })
    .await;
}

#[test]
fn the_countdown_wakes_on_each_whole_second_and_then_the_deadline() {
    let now = Instant::now();
    let deadline = now + Duration::from_millis(2500);
    assert_eq!(whole_seconds(deadline - now), 3);
    assert_eq!(next_second(deadline, now), now + Duration::from_millis(500));
    let last = now + Duration::from_millis(1500);
    assert_eq!(next_second(deadline, last), deadline);
    assert_eq!(next_second(deadline, deadline), deadline);
    assert_eq!(whole_seconds(Duration::ZERO), 0);
}

mod capture;
mod desktop;
mod per_run;
mod playback;
mod positions;
mod recording;
mod settings;
