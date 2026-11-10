//! The daemon's controller: the one owner of what the program is doing.
//!
//! The tray, the session watcher and signal handlers send it [`Message`]s; the
//! hotkey reader sends it hotkey events. Nothing else starts or stops a run,
//! opens the input devices or changes the mode. After every change the
//! controller publishes a [`View`] for the tray to show.
//!
//! The controller runs as one future on the daemon's single thread. The
//! clicking run and the hotkey reader are futures it polls in its own loop,
//! so whatever happens to the controller happens to them. Every wait it has
//! (the tray countdown, the wait for the hotkey's modifiers to come up) is a
//! branch of that loop's one `select!`, beside every message, so a stop or a
//! shutdown ends it at once.

mod hotkeys;
mod session;
mod view;

use std::future::{Future, pending, poll_fn};
use std::mem;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, sleep_until};

pub use hotkeys::{HotkeyReader, HotkeySender};
pub use session::{Blocked, SessionFacts};
pub use view::{Activity, View, describe};

use crate::clicker::{Ended, RunEnd, Signals, run};
use crate::config::Config;
use crate::hotkeys::{Action, HotkeyEvent, HotkeyStatus};
use crate::input::{Input, InputBackend, InputError};
use hotkeys::HotkeySource;

/// How long a start from the hotkey waits for the hotkey's modifiers to come
/// up before it is cancelled.
pub const RELEASE_WAIT: Duration = Duration::from_secs(30);

/// What the tray, the session watcher and signal handlers ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Start clicking after the tray countdown. Ignored unless idle.
    Start,
    /// Stop clicking, or cancel a start that has not begun. Ignored when idle.
    Stop,
    /// Stop everything and end the controller: the tray's Quit, SIGTERM or
    /// SIGINT.
    Quit,
    /// The session's state changed: why clicking may not run, or `None` when
    /// it may. Anything but `None` stops clicking.
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

/// What the controller needs from the platform: its input devices and a
/// hotkey reader that ignores them.
pub trait Platform {
    type Backend: InputBackend + 'static;

    /// Opens the virtual input devices.
    fn open_input(&mut self) -> Result<Input<Self::Backend>, InputError>;

    /// Starts reading hotkeys, ignoring the devices of `own` (none when it is
    /// `None`), and sending to `send`. The controller drops the reader before
    /// it opens devices again, so no reader runs while devices it does not
    /// know about exist.
    fn read_hotkeys(
        &mut self,
        own: Option<&Input<Self::Backend>>,
        send: HotkeySender,
    ) -> HotkeyReader;
}

/// The controller's mode. Only the controller changes it.
enum State<B: InputBackend> {
    Idle,
    /// A start from the tray, clicking at this deadline.
    Countdown(Instant),
    /// A start from the hotkey, waiting for its modifiers to come up until
    /// this deadline.
    Release(Instant),
    Clicking(Running<B>),
}

/// A clicking run and the signals that end it.
struct Running<B: InputBackend> {
    run: Pin<Box<dyn Future<Output = RunEnd<B>>>>,
    stop: Option<oneshot::Sender<()>>,
    shutdown: Option<oneshot::Sender<()>>,
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
    /// The devices while no run has them. `None` during a run, and after
    /// opening them again failed.
    input: Option<Input<P::Backend>>,
    /// Why the devices could not be opened again.
    input_error: Option<String>,
    hotkeys: HotkeySource,
    hotkey_status: Option<HotkeyStatus>,
    /// Starts out unknown: a state not yet read is not "unlocked".
    blocked: Option<Blocked>,
    notice: Option<String>,
    /// A [`Message::Sleep`] handled but not yet reported.
    sleep_notice: Option<u64>,
    released_for_sleep: watch::Sender<u64>,
    rng: fastrand::Rng,
}

impl<P: Platform> Controller<P> {
    /// A controller over `input`, the devices opened at start, and how to
    /// reach it.
    pub fn new(
        platform: P,
        input: Input<P::Backend>,
        config: Config,
        config_error: Option<String>,
    ) -> (Controller<P>, Links) {
        let (send, inbox) = mpsc::unbounded_channel();
        let (released_for_sleep, sleep_reports) = watch::channel(0);
        let (view, shown) = watch::channel(View {
            activity: Activity::Idle,
            settings: config.clicker,
            hotkey: config.hotkeys.toggle_clicking,
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
            input: Some(input),
            input_error: None,
            hotkeys: HotkeySource::none(),
            hotkey_status: None,
            blocked: Some(Blocked::Unknown("not read yet".to_owned())),
            notice: None,
            sleep_notice: None,
            released_for_sleep,
            rng: fastrand::Rng::new(),
        };
        let links = Links {
            send,
            view: shown,
            released_for_sleep: sleep_reports,
        };
        (controller, links)
    }

    /// Runs until [`Message::Quit`], or until every sender of the inbox is
    /// gone. A run still going then ends with the shutdown signal and is
    /// waited for, so nothing stays pressed.
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
            ..
        } = self;
        tokio::select! {
            biased;
            message = inbox.recv() => Wake::Message(message.unwrap_or(Message::Quit)),
            event = hotkeys::received(&mut hotkeys.events) => Wake::Hotkey(event),
            end = run_end(state) => Wake::RunEnded(end),
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
        let end = match &mut self.state {
            State::Clicking(running) if running.stopping() => (&mut running.run).await,
            _ => return,
        };
        self.run_ended(end);
    }

    /// Handles one wake; `false` when the controller should end.
    fn handle(&mut self, wake: Wake<P::Backend>) -> bool {
        match wake {
            Wake::Message(message) => return self.message(message),
            Wake::RunEnded(end) => self.run_ended(end),
            Wake::Hotkey(event) => self.hotkey(event),
            Wake::Deadline => self.deadline(),
        }
        true
    }

    /// Handles one message; `false` for [`Message::Quit`].
    fn message(&mut self, message: Message) -> bool {
        match message {
            Message::Start => self.start_from_tray(),
            Message::Stop => self.stop(None),
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
            // Recording and playback come with M2.
            HotkeyEvent::Fired(Action::ToggleRecording | Action::TogglePlayback) => {}
            HotkeyEvent::ModifiersReleased => self.released(),
            HotkeyEvent::ReleaseLost => self.cancel_release(format!(
                "Not started: could not tell when the hotkey was let go; press {} again",
                self.config.hotkeys.toggle_clicking
            )),
            HotkeyEvent::Status(status) => self.hotkey_status_changed(status),
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
            State::Countdown(_) | State::Release(_) => self.state = State::Idle,
            State::Clicking(running) => running.stop(),
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

    /// Stops a run or cancels a start, and says why when `notice` is given.
    fn stop(&mut self, notice: Option<String>) {
        match &mut self.state {
            State::Idle => return,
            State::Countdown(_) | State::Release(_) => self.state = State::Idle,
            State::Clicking(running) => running.stop(),
        }
        if notice.is_some() {
            self.notice = notice;
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
        match &self.blocked {
            Some(reason) => {
                self.notice = Some(format!("Not started: {reason}"));
                false
            }
            None => true,
        }
    }

    fn deadline(&mut self) {
        match self.state {
            State::Countdown(deadline) if Instant::now() >= deadline => self.begin_run(),
            // Its deadline is the only time a release wait wakes by itself.
            State::Release(_) => self.release_expired(),
            // A second of the countdown passed; the view shows it.
            _ => {}
        }
    }

    /// The next time the controller must wake by itself: the end of a wait,
    /// or the next whole second of the countdown.
    fn next_deadline(&self) -> Option<Instant> {
        match self.state {
            State::Countdown(deadline) => Some(next_second(deadline, Instant::now())),
            State::Release(deadline) => Some(deadline),
            State::Idle | State::Clicking(_) => None,
        }
    }

    /// Starts clicking, opening the devices again first if a run lost them.
    fn begin_run(&mut self) {
        self.state = State::Idle;
        if self.input.is_none() {
            self.reopen();
        }
        let Some(input) = self.input.take() else {
            return;
        };
        self.notice = None;
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
        });
    }

    fn run_ended(&mut self, end: RunEnd<P::Backend>) {
        self.state = State::Idle;
        match end.result {
            Ok(Ended::Finished) => self.notice = Some("Finished every action".to_owned()),
            Ok(Ended::Stopped | Ended::ShutDown) => {}
            Err(err) => self.notice = Some(format!("Clicking stopped: {err}")),
        }
        match end.input {
            Some(input) => self.input = Some(input),
            None => self.reopen(),
        }
    }

    /// Opens the devices again after a run lost them, and restarts the
    /// hotkey reader with their identities. The old reader goes first: it
    /// would read the new devices as keyboards.
    fn reopen(&mut self) {
        self.hotkeys = HotkeySource::none();
        match self.platform.open_input() {
            Ok(input) => {
                self.input = Some(input);
                self.input_error = None;
            }
            Err(err) => self.input_error = Some(err.to_string()),
        }
        self.start_reader();
    }

    fn start_reader(&mut self) {
        let Controller {
            platform, input, ..
        } = self;
        self.hotkeys = HotkeySource::start(|send| platform.read_hotkeys(input.as_ref(), send));
        self.hotkey_status = None;
    }

    fn publish(&self) {
        let view = View {
            activity: self.activity(),
            settings: self.config.clicker,
            hotkey: self.config.hotkeys.toggle_clicking,
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
            State::Clicking(_) => Activity::Clicking,
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

/// The end of the clicking run; never resolves while nothing runs.
fn run_end<B: InputBackend>(state: &mut State<B>) -> impl Future<Output = RunEnd<B>> + '_ {
    poll_fn(move |cx| match state {
        State::Clicking(running) => running.run.as_mut().poll(cx),
        _ => Poll::Pending,
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
