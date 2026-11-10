//! The daemon's controller: the one owner of what the program is doing.
//!
//! The tray, the session watcher and signal handlers send it [`Message`]s; the
//! input reader sends it hotkey events. Nothing else starts or stops a run or
//! a recording, opens the input devices or changes the mode. After every
//! change the controller publishes a [`View`] for the tray to show.
//!
//! The controller runs as one future on the daemon's single thread. The
//! clicking run, the hotkey reader and an open of the input devices are
//! futures it polls in its own loop, so whatever happens to the controller
//! happens to them. Every wait it has (the tray countdown, the wait for the
//! hotkey's modifiers to come up, an open that waits on the desktop's
//! permission dialog) is a branch of that loop's one `select!`, beside every
//! message, so a stop or a shutdown ends it at once.

mod desktop;
mod hotkeys;
mod recording;
mod session;
mod view;

use std::future::{Future, pending, poll_fn};
use std::mem;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, sleep_until};

pub use hotkeys::{HotkeyReader, HotkeySender};
pub use session::{Blocked, SessionFacts};
pub use view::{Activity, Hotkey, View, describe};

use crate::clicker::{Ended, RunEnd, Signals, run};
use crate::config::Config;
use crate::hotkeys::{Action, HotkeyEvent, HotkeyStatus, Unavailable};
use crate::input::{Input, InputBackend, InputError};
use crate::recording::{LocalTime, RecordingFeed, RecordingSlot};
use hotkeys::HotkeySource;

/// How long a start from the hotkey waits for the hotkey's modifiers to come
/// up before it is cancelled.
pub const RELEASE_WAIT: Duration = Duration::from_secs(30);

/// What the tray, the session watcher and signal handlers ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Start clicking after the tray countdown. Ignored unless idle.
    Start,
    /// Stop clicking, or cancel a start that has not begun. Ignored unless
    /// clicking or about to.
    Stop,
    /// Start recording after the tray countdown. Ignored unless idle.
    Record,
    /// Stop and save the recording, trimming the trip to the tray, or cancel
    /// a recording that has not begun.
    StopRecording,
    /// The tray menu opened: a stop from it is on its way.
    MenuOpened,
    /// Stop everything and end the controller: the tray's Quit, SIGTERM or
    /// SIGINT.
    Quit,
    /// The session's state changed: why clicking and recording may not run,
    /// or `None` when they may. Anything but `None` stops clicking, and
    /// stops and saves a recording.
    Session(Option<Blocked>),
    /// logind asked the session to lock: stop now. The lock itself arrives
    /// as [`Message::Session`].
    LockRequested,
    /// The machine is about to sleep; the n-th such notice. Stops clicking
    /// and refuses starts like [`Blocked::Sleeping`], until a
    /// [`Message::Session`] says otherwise. Once nothing is held any more,
    /// the controller reports `n` on [`Links::released_for_sleep`], so the
    /// sender can let the machine sleep.
    Sleep(u64),
}

/// Where to reach a controller, and what it reports.
pub struct Links {
    /// Where to send it messages.
    pub send: mpsc::UnboundedSender<Message>,
    /// What it shows.
    pub view: watch::Receiver<View>,
    /// The last [`Message::Sleep`] it handled, reported once nothing was
    /// held any more. Starts at 0.
    pub released_for_sleep: watch::Receiver<u64>,
}

/// Opening the input devices: it may wait on the user (a permission
/// dialog), so the controller polls it beside everything else, and drops it
/// to cancel.
pub type OpenInput<B> = Pin<Box<dyn Future<Output = Result<Input<B>, InputError>>>>;

/// Resolves when the input devices stopped working without a write saying
/// so: the desktop ended the session. Never resolves on a backend that
/// cannot tell.
pub type InputLoss = Pin<Box<dyn Future<Output = InputError>>>;

/// How long a platform's input devices live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputLifetime {
    /// Opened once and kept between runs. They are input devices the hotkey
    /// reader can see, so the reader stops before each open and starts again
    /// after it, knowing them (the kernel backend).
    Kept,
    /// Opened for each run and closed when it ends. The hotkey reader cannot
    /// see them and keeps running while one opens (the portal backend: the
    /// desktop shows that remote input is active only while a run lasts).
    PerRun,
}

/// Where a platform's input reader gets its hotkeys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputReader {
    /// From the keyboards: it reads device nodes, tells the program's own
    /// devices from the user's, and can feed a recording.
    Devices,
    /// From the desktop's shortcuts: it reads no device, so it runs through
    /// every open, cannot feed a recording, and cannot tell the program's
    /// own key output from the user's. It reports the desktop's triggers.
    Desktop,
}

/// What the controller needs from the platform: its input devices, an input
/// reader that ignores them, and the clocks and directory recordings need.
pub trait Platform {
    type Backend: InputBackend + 'static;

    /// How long the devices live.
    const LIFETIME: InputLifetime;

    /// Opens the input devices. The future owns what it opened so far:
    /// dropping it part way closes that again.
    fn open_input(&mut self) -> OpenInput<Self::Backend>;

    /// Watches `input` for a loss that no write reports, such as the desktop
    /// ending the session while a button is held. The default never fires.
    fn input_lost(&self, input: &Input<Self::Backend>) -> InputLoss {
        let _ = input;
        Box::pin(pending())
    }

    /// Where the input reader gets its hotkeys.
    fn input_reader(&self) -> InputReader {
        InputReader::Devices
    }

    /// Starts reading input, ignoring the devices of `own` (none when it is
    /// `None`): hotkeys go to `send`, and while a recording runs, what the
    /// devices do goes to `feed`. The controller drops a reader of devices
    /// before it opens kept devices again, so no such reader runs while
    /// devices it does not know about exist.
    ///
    /// When a recording starts, the reader reads what every device holds
    /// and tells the recorder, so a key pressed while the recording was
    /// being started counts as held from the start.
    fn read_input(
        &mut self,
        own: Option<&Input<Self::Backend>>,
        send: HotkeySender,
        feed: RecordingFeed,
    ) -> HotkeyReader;

    /// Now, on the clock input events are stamped with (the monotonic
    /// clock).
    fn event_clock(&self) -> Duration;

    /// The local date and time now, which names a new recording; `None`
    /// when it cannot be read.
    fn local_time(&self) -> Option<LocalTime>;

    /// Where recordings are saved; `None` when there is no data directory.
    fn recordings_dir(&self) -> Option<PathBuf>;
}

/// The controller's mode. Only the controller changes it.
enum State<B: InputBackend> {
    Idle,
    /// A start from the tray, clicking at this deadline.
    Countdown(Instant),
    /// A start from the hotkey, waiting for its modifiers to come up until
    /// this deadline.
    Release(Instant),
    /// A start waiting for the devices to open.
    Opening,
    Clicking(Running<B>),
    /// A recording from the tray starts at this deadline.
    RecordCountdown(Instant),
    /// Recording; it stops at its time limit, this deadline.
    Recording(Instant),
}

/// A clicking run and the signals that end it.
struct Running<B: InputBackend> {
    run: Pin<Box<dyn Future<Output = RunEnd<B>>>>,
    stop: Option<oneshot::Sender<()>>,
    shutdown: Option<oneshot::Sender<()>>,
    /// Fires when the devices are lost without a write noticing.
    loss: InputLoss,
    /// Why the devices were lost, once `loss` fired.
    lost: Option<InputError>,
}

impl<B: InputBackend> Running<B> {
    fn stop(&mut self) {
        fire(&mut self.stop);
    }

    /// Whether the run was told to stop.
    fn stopping(&self) -> bool {
        self.stop.is_none()
    }

    /// Ends the run with the shutdown signal and waits for it to hand back
    /// what it held, released. The engine ends at once on the signal.
    async fn shut_down(mut self) -> RunEnd<B> {
        fire(&mut self.shutdown);
        self.run.await
    }
}

/// Fires a signal once; later calls do nothing.
fn fire(signal: &mut Option<oneshot::Sender<()>>) {
    if let Some(sender) = signal.take() {
        // The receiver is gone only when the run has already ended.
        let _ = sender.send(());
    }
}

/// A signal for the engine: fires when its sender fires, or is dropped.
async fn signal(receiver: oneshot::Receiver<()>) {
    // A dropped sender ends the run too; either way, it ends.
    let _ = receiver.await;
}

/// What woke the controller.
enum Wake<B: InputBackend> {
    Message(Message),
    RunEnded(RunEnd<B>),
    InputLost(InputError),
    Opened(Result<Input<B>, InputError>),
    Hotkey(HotkeyEvent),
    Deadline,
}

/// The daemon's controller. Build it with [`Controller::new`] and drive it
/// with [`Controller::run`].
pub struct Controller<P: Platform> {
    platform: P,
    config: Config,
    config_error: Option<String>,
    inbox: mpsc::UnboundedReceiver<Message>,
    view: watch::Sender<View>,
    state: State<P::Backend>,
    /// The devices while no run has them. `None` during a run, after
    /// opening them again failed, and between runs when they live per run.
    input: Option<Input<P::Backend>>,
    /// The devices being opened.
    opening: Option<OpenInput<P::Backend>>,
    /// Why the devices could not be opened again.
    input_error: Option<String>,
    hotkeys: HotkeySource,
    hotkey_status: Option<HotkeyStatus>,
    /// The desktop's hotkeys, when it delivers them.
    desktop: Option<desktop::DesktopHotkeys>,
    /// Starts out unknown: a state not yet read is not "unlocked".
    blocked: Option<Blocked>,
    notice: Option<String>,
    /// A [`Message::Sleep`] handled but not yet reported.
    sleep_notice: Option<u64>,
    released_for_sleep: watch::Sender<u64>,
    rng: fastrand::Rng,
    /// The recording in progress, shared with the input reader.
    recording: RecordingSlot,
    /// When the tray menu last opened during the recording, on the event
    /// clock.
    menu_opened: Option<Duration>,
}

impl<P: Platform> Controller<P> {
    /// A controller over `input`, the devices opened at start (`None` when
    /// they open for each run), and how to reach it.
    pub fn new(
        platform: P,
        input: Option<Input<P::Backend>>,
        config: Config,
        config_error: Option<String>,
    ) -> (Controller<P>, Links) {
        let (send, inbox) = mpsc::unbounded_channel();
        let (released_for_sleep, sleep_reports) = watch::channel(0);
        let desktop = Self::first_triggers(&platform);
        let (view, shown) = watch::channel(View {
            activity: Activity::Idle,
            settings: config.clicker,
            hotkey: desktop::shown_hotkey(config.hotkeys, desktop.as_ref(), Action::ToggleClicking),
            record_hotkey: desktop::shown_hotkey(
                config.hotkeys,
                desktop.as_ref(),
                Action::ToggleRecording,
            ),
            notice: None,
            problems: Vec::new(),
        });
        let controller = Controller {
            platform,
            config,
            config_error,
            inbox,
            view,
            state: State::Idle,
            input,
            opening: None,
            input_error: None,
            hotkeys: HotkeySource::none(),
            hotkey_status: None,
            desktop,
            blocked: Some(Blocked::Unknown("not read yet".to_owned())),
            notice: None,
            sleep_notice: None,
            released_for_sleep,
            rng: fastrand::Rng::new(),
            recording: RecordingSlot::new(),
            menu_opened: None,
        };
        let links = Links {
            send,
            view: shown,
            released_for_sleep: sleep_reports,
        };
        (controller, links)
    }

    /// Shows `err` as a problem until the devices open: what a check at
    /// start found, before any open.
    pub fn report_input_problem(&mut self, err: &InputError) {
        self.input_error = Some(err.to_string());
    }

    /// Runs until [`Message::Quit`], or until every sender of the inbox is
    /// gone. A run still going then ends with the shutdown signal and is
    /// waited for, so nothing stays pressed; a recording is stopped and
    /// saved.
    pub async fn run(mut self) {
        self.start_reader();
        loop {
            self.publish();
            let wake = self.next_wake().await;
            if !self.handle(wake) {
                break;
            }
            self.finish_stopped_run().await;
            self.report_sleep();
        }
        self.stop_recording(None);
        if let State::Clicking(running) = mem::replace(&mut self.state, State::Idle) {
            drop(running.shut_down().await);
        }
    }

    /// The next thing to handle. Branches are tried in order, so what was
    /// asked for comes before what it would prevent: a queued Stop or lock
    /// before the hotkey events, the run's next action and the deadlines;
    /// hotkey events (a stop) before the run. The reader goes last: kept busy
    /// by a flood of key events, it spends tokio's budget, and anything
    /// polled after it would wait. Everything else is cheap to poll. (A due
    /// timer and a queued message cannot be made ready in one poll on the
    /// paused clock, so the tests pin the reader's place and the outcomes,
    /// not every tie.)
    async fn next_wake(&mut self) -> Wake<P::Backend> {
        let deadline = self.next_deadline();
        let Controller {
            inbox,
            state,
            hotkeys,
            opening,
            ..
        } = self;
        tokio::select! {
            biased;
            message = inbox.recv() => Wake::Message(message.unwrap_or(Message::Quit)),
            event = hotkeys::received(&mut hotkeys.events) => Wake::Hotkey(event),
            wake = run_wake(state) => wake,
            opened = open_end(opening) => Wake::Opened(opened),
            () = sleep_until_some(deadline) => Wake::Deadline,
            never = hotkeys::drive(&mut hotkeys.reader) => match never {},
        }
    }

    /// Reports a sleep notice once its stop is done: by now any run it
    /// stopped has ended and let go of everything.
    fn report_sleep(&mut self) {
        if let Some(notice) = self.sleep_notice.take() {
            self.released_for_sleep.send_replace(notice);
        }
    }

    /// A run told to stop ends at once (the engine's contract), so it is
    /// waited for right here: a stopped run never waits behind other work,
    /// and no other state sees a run that is still stopping.
    async fn finish_stopped_run(&mut self) {
        let (end, lost) = match &mut self.state {
            State::Clicking(running) if running.stopping() => {
                ((&mut running.run).await, running.lost.take())
            }
            _ => return,
        };
        self.run_ended(end, lost);
    }

    /// Handles one wake; `false` when the controller should end.
    fn handle(&mut self, wake: Wake<P::Backend>) -> bool {
        match wake {
            Wake::Message(message) => return self.message(message),
            Wake::RunEnded(end) => self.run_ended(end, None),
            Wake::InputLost(err) => self.devices_lost(err),
            Wake::Opened(opened) => self.opened(opened),
            Wake::Hotkey(event) => self.hotkey(event),
            Wake::Deadline => self.deadline(),
        }
        true
    }

    /// Handles one message; `false` for [`Message::Quit`].
    fn message(&mut self, message: Message) -> bool {
        match message {
            Message::Start => self.start_from_tray(),
            Message::Stop => self.stop_clicking(),
            Message::Record => self.record_from_tray(),
            Message::StopRecording => self.stop_recording_from_tray(),
            Message::MenuOpened => self.menu_opened(),
            Message::Session(blocked) => self.session(blocked),
            Message::LockRequested => self.stop(Some(format!("Stopped: {}", Blocked::Locked))),
            Message::Sleep(notice) => {
                self.session(Some(Blocked::Sleeping));
                self.sleep_notice = Some(notice);
            }
            Message::Quit => return false,
        }
        true
    }

    fn hotkey(&mut self, event: HotkeyEvent) {
        match event {
            HotkeyEvent::Fired(Action::ToggleClicking) => self.toggle(),
            HotkeyEvent::Fired(Action::ToggleRecording) => self.toggle_recording(),
            // Playback comes with R02.
            HotkeyEvent::Fired(Action::TogglePlayback) => {}
            HotkeyEvent::ModifiersReleased => self.released(),
            HotkeyEvent::ReleaseLost => self.cancel_release(format!(
                "Not started: could not tell when the hotkey was let go; press {} again",
                self.config.hotkeys.toggle_clicking
            )),
            HotkeyEvent::Status(status) => self.hotkey_status_changed(status),
            HotkeyEvent::Triggers(triggers) => self.triggers_changed(triggers),
        }
    }

    /// The modifiers came up: a pending hotkey start begins, unless its wait
    /// ran out first. A late controller can see the release and the deadline
    /// at once; the release is handled first, so it checks the time itself.
    fn released(&mut self) {
        let State::Release(deadline) = self.state else {
            return;
        };
        if Instant::now() < deadline {
            self.begin_run();
        } else {
            self.release_expired();
        }
    }

    fn release_expired(&mut self) {
        self.state = State::Idle;
        self.notice = Some(format!(
            "Not started: the hotkey was held for {} s; let go of it to start",
            RELEASE_WAIT.as_secs()
        ));
    }

    fn hotkey_status_changed(&mut self, status: HotkeyStatus) {
        let unavailable = matches!(status, HotkeyStatus::Unavailable(_));
        self.desktop_status_changed(&status);
        if let HotkeyStatus::Unavailable(why @ Unavailable::WatchFailed(_)) = &status {
            self.input_lost(why.to_string());
        }
        self.hotkey_status = Some(status);
        if unavailable {
            self.cancel_release("Not started: hotkeys stopped working".to_owned());
        }
    }

    /// The hotkey: starts after its modifiers come up, cancels a start, or
    /// stops a run.
    fn toggle(&mut self) {
        match &mut self.state {
            State::Idle => {
                if self.may_start()
                    && let Some(deadline) = Instant::now().checked_add(RELEASE_WAIT)
                {
                    self.state = State::Release(deadline);
                }
            }
            State::Countdown(_) | State::Release(_) | State::Opening => self.cancel_start(),
            State::Clicking(running) => running.stop(),
            // Other hotkeys do nothing while recording.
            State::RecordCountdown(_) | State::Recording(_) => {}
        }
    }

    fn start_from_tray(&mut self) {
        if !matches!(self.state, State::Idle) || !self.may_start() {
            return;
        }
        let delay = self.config.tray_start_delay;
        if delay.is_zero() {
            self.begin_run();
        } else if let Some(deadline) = Instant::now().checked_add(delay) {
            // The config caps the delay at an hour, so the deadline exists.
            self.state = State::Countdown(deadline);
        }
    }

    /// The tray's Stop clicking: stops a run or cancels its start; nothing
    /// else.
    fn stop_clicking(&mut self) {
        if matches!(
            self.state,
            State::Countdown(_) | State::Release(_) | State::Opening | State::Clicking(_)
        ) {
            self.stop(None);
        }
    }

    /// Stops a run, a recording (saving it) or a start, and says why when
    /// `notice` is given.
    fn stop(&mut self, notice: Option<String>) {
        match &mut self.state {
            State::Idle => return,
            State::Countdown(_) | State::Release(_) | State::Opening => self.cancel_start(),
            State::RecordCountdown(_) => self.state = State::Idle,
            State::Clicking(running) => running.stop(),
            State::Recording(_) => return self.stop_recording(notice),
        }
        if notice.is_some() {
            self.notice = notice;
        }
    }

    /// Cancels a start that has not begun. An open made for it alone is
    /// dropped, which closes what it opened so far (a permission dialog
    /// included); kept devices go on opening.
    fn cancel_start(&mut self) {
        self.state = State::Idle;
        if P::LIFETIME == InputLifetime::PerRun {
            self.opening = None;
        }
    }

    fn cancel_release(&mut self, notice: String) {
        if matches!(self.state, State::Release(_)) {
            self.state = State::Idle;
            self.notice = Some(notice);
        }
    }

    fn session(&mut self, blocked: Option<Blocked>) {
        if let Some(reason) = &blocked {
            self.stop(Some(format!("Stopped: {reason}")));
        }
        self.blocked = blocked;
    }

    /// Whether a start may begin; says why not when it may not.
    fn may_start(&mut self) -> bool {
        self.session_allows("Not started") && self.keys_allow()
    }

    /// Whether the session lets anything start; when it does not, the
    /// notice says why, after `refused`.
    fn session_allows(&mut self, refused: &str) -> bool {
        match &self.blocked {
            Some(reason) => {
                self.notice = Some(format!("{refused}: {reason}"));
                false
            }
            None => true,
        }
    }

    fn deadline(&mut self) {
        let now = Instant::now();
        match self.state {
            State::Countdown(deadline) if now >= deadline => self.begin_run(),
            // Its deadline is the only time a release wait wakes by itself.
            State::Release(_) => self.release_expired(),
            State::RecordCountdown(deadline) if now >= deadline => self.begin_recording(None),
            State::Recording(limit) if now >= limit => self.recording_limit_reached(),
            // A second of a countdown passed; the view shows it.
            _ => {}
        }
    }

    /// The next time the controller must wake by itself: the end of a wait,
    /// the next whole second of a countdown, or a recording's time limit.
    fn next_deadline(&self) -> Option<Instant> {
        match self.state {
            State::Countdown(deadline) | State::RecordCountdown(deadline) => {
                Some(next_second(deadline, Instant::now()))
            }
            State::Release(deadline) | State::Recording(deadline) => Some(deadline),
            State::Idle | State::Opening | State::Clicking(_) => None,
        }
    }

    /// Starts clicking, or waits for the devices to open first when no
    /// devices are open.
    fn begin_run(&mut self) {
        match self.input.take() {
            Some(input) => self.run_with(input),
            None => {
                self.state = State::Opening;
                if self.opening.is_none() {
                    self.open();
                }
            }
        }
    }

    fn run_with(&mut self, input: Input<P::Backend>) {
        self.notice = None;
        let loss = self.platform.input_lost(&input);
        let (stop, stop_signal) = oneshot::channel();
        let (shutdown, shutdown_signal) = oneshot::channel();
        let settings = self.config.clicker;
        let rng = self.rng.fork();
        let signals = Signals {
            stop: signal(stop_signal),
            shutdown: signal(shutdown_signal),
        };
        self.state = State::Clicking(Running {
            run: Box::pin(async move { run(input, &settings, rng, signals).await }),
            stop: Some(stop),
            shutdown: Some(shutdown),
            loss,
            lost: None,
        });
    }

    /// The devices stopped working under a run: it stops at once, and its
    /// end reports why.
    fn devices_lost(&mut self, err: InputError) {
        if let State::Clicking(running) = &mut self.state {
            running.lost = Some(err);
            running.stop();
        }
    }

    /// `lost` says why the devices were lost under the run, if they were.
    fn run_ended(&mut self, end: RunEnd<P::Backend>, lost: Option<InputError>) {
        self.state = State::Idle;
        let devices_lost = lost.is_some();
        match (lost, end.result) {
            (Some(err), _) | (None, Err(err)) => {
                self.notice = Some(format!("Clicking stopped: {err}"));
            }
            (None, Ok(Ended::Finished)) => self.notice = Some("Finished every action".to_owned()),
            (None, Ok(Ended::Stopped | Ended::ShutDown)) => {}
        }
        // Lost devices are closed, and so are devices that live per run.
        let kept = end
            .input
            .filter(|_| !devices_lost && P::LIFETIME == InputLifetime::Kept);
        match kept {
            Some(input) => self.input = Some(input),
            None if P::LIFETIME == InputLifetime::Kept => self.open(),
            None => {}
        }
    }

    /// Starts opening the devices. Kept devices are visible to a reader of
    /// devices, so it stops first (it would read them as keyboards) and
    /// starts again once the open has ended.
    fn open(&mut self) {
        if self.reader_restarts() {
            self.hotkeys = HotkeySource::none();
        }
        self.opening = Some(self.platform.open_input());
    }

    /// An open ended: a start waiting for it begins, or says why it cannot.
    fn opened(&mut self, opened: Result<Input<P::Backend>, InputError>) {
        self.opening = None;
        let waiting = matches!(self.state, State::Opening);
        if waiting {
            self.state = State::Idle;
        }
        self.keep_opened(opened, waiting);
        if self.reader_restarts() {
            self.start_reader();
        }
        if waiting && let Some(input) = self.input.take() {
            self.run_with(input);
        }
    }

    /// Keeps what an open gave, or says why it failed. Per-run devices with
    /// no start waiting for them close again at once.
    fn keep_opened(&mut self, opened: Result<Input<P::Backend>, InputError>, waiting: bool) {
        let kept = P::LIFETIME == InputLifetime::Kept;
        match opened {
            Ok(input) => {
                self.input_error = None;
                if waiting || kept {
                    self.input = Some(input);
                }
            }
            Err(err) => {
                if waiting {
                    self.notice = Some(format!("Not started: {err}"));
                }
                if kept {
                    self.input_error = Some(err.to_string());
                }
            }
        }
    }

    fn start_reader(&mut self) {
        let Controller {
            platform,
            input,
            recording,
            ..
        } = self;
        let feed = recording.feed();
        self.hotkeys = HotkeySource::start(|send| platform.read_input(input.as_ref(), send, feed));
        self.hotkey_status = None;
    }

    fn publish(&self) {
        let view = View {
            activity: self.activity(),
            settings: self.config.clicker,
            hotkey: self.shown_hotkey(Action::ToggleClicking),
            record_hotkey: self.shown_hotkey(Action::ToggleRecording),
            notice: self.notice.clone(),
            problems: self.problems(),
        };
        self.view.send_if_modified(|shown| {
            let changed = *shown != view;
            if changed {
                *shown = view;
            }
            changed
        });
    }

    fn activity(&self) -> Activity {
        match self.state {
            State::Idle => Activity::Idle,
            State::Countdown(deadline) => Activity::StartingIn(whole_seconds(
                deadline.saturating_duration_since(Instant::now()),
            )),
            State::Release(_) => Activity::WaitingForRelease,
            State::Opening => Activity::Opening,
            State::Clicking(_) => Activity::Clicking,
            State::RecordCountdown(deadline) => Activity::RecordingIn(whole_seconds(
                deadline.saturating_duration_since(Instant::now()),
            )),
            State::Recording(_) => Activity::Recording,
        }
    }

    fn problems(&self) -> Vec<String> {
        let hotkeys = match &self.hotkey_status {
            Some(HotkeyStatus::Unavailable(why)) => Some(why.to_string()),
            _ => None,
        };
        let blocked = self
            .blocked
            .as_ref()
            .map(|reason| format!("Clicking cannot start: {reason}"));
        let input = self
            .input_error
            .as_ref()
            .map(|err| format!("Clicking cannot start: {err}"));
        [self.config_error.clone(), hotkeys, blocked, input]
            .into_iter()
            .flatten()
            .collect()
    }
}

/// The end of the clicking run, or else the loss of its devices once the
/// run's own writes have not reported it; never resolves while nothing runs.
fn run_wake<B: InputBackend>(state: &mut State<B>) -> impl Future<Output = Wake<B>> + '_ {
    poll_fn(move |cx| {
        let State::Clicking(running) = state else {
            return Poll::Pending;
        };
        if let Poll::Ready(end) = running.run.as_mut().poll(cx) {
            return Poll::Ready(Wake::RunEnded(end));
        }
        if running.lost.is_none() {
            return running.loss.as_mut().poll(cx).map(Wake::InputLost);
        }
        Poll::Pending
    })
}

/// The end of the open in flight; never resolves while none is.
fn open_end<B: InputBackend>(
    opening: &mut Option<OpenInput<B>>,
) -> impl Future<Output = Result<Input<B>, InputError>> + '_ {
    poll_fn(move |cx| match opening {
        Some(open) => open.as_mut().poll(cx),
        None => Poll::Pending,
    })
}

async fn sleep_until_some(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => pending().await,
    }
}

/// Whole seconds in `left`, rounded up.
fn whole_seconds(left: Duration) -> u64 {
    left.as_secs()
        .saturating_add(u64::from(left.subsec_nanos() > 0))
}

/// When the countdown to `deadline` next shows one second less: the next
/// whole second before `deadline`, or `deadline` itself in the last second.
fn next_second(deadline: Instant, now: Instant) -> Instant {
    let shown = whole_seconds(deadline.saturating_duration_since(now));
    let after = Duration::from_secs(shown.saturating_sub(1));
    deadline.checked_sub(after).unwrap_or(deadline)
}

#[cfg(test)]
mod tests;
