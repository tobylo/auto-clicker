//! The controller over devices that open for each run and can be lost
//! without a write noticing, as on the portal backend. Opens wait until the
//! test lets them finish, as a permission dialog does.

use tokio::sync::oneshot;

use super::*;

/// One open the test decides: it resolves with what the test sends.
type OpenGate = oneshot::Sender<Result<(), InputError>>;

#[derive(Default)]
struct Gates {
    /// The open in flight, waiting for the test.
    open: Option<OpenGate>,
    /// Fires the loss of the devices opened last.
    loss: Option<oneshot::Sender<InputError>>,
    /// How many opens were dropped before they ended.
    opens_dropped: usize,
    /// How many devices were closed (dropped).
    closed: usize,
}

#[derive(Clone, Default)]
struct PerRunShared {
    devices: Shared,
    gates: Arc<Mutex<Gates>>,
}

impl PerRunShared {
    fn gates<R>(&self, f: impl FnOnce(&mut Gates) -> R) -> R {
        f(&mut self.gates.lock().unwrap())
    }

    /// Ends the open in flight with `result`.
    fn finish_open(&self, result: Result<(), InputError>) {
        let gate = self.gates(|g| g.open.take()).expect("an open in flight");
        let _ = gate.send(result);
    }

    fn open_in_flight(&self) -> bool {
        self.gates(|g| g.open.as_ref().is_some_and(|gate| !gate.is_closed()))
    }

    fn lose(&self, err: InputError) {
        let loss = self.gates(|g| g.loss.take()).expect("devices to lose");
        let _ = loss.send(err);
    }
}

/// Notes a drop, unless disarmed: of an open that never ended, or of
/// devices.
struct DropCount(Option<PerRunShared>, fn(&mut Gates));

impl DropCount {
    fn new(shared: PerRunShared, count: fn(&mut Gates)) -> DropCount {
        DropCount(Some(shared), count)
    }

    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for DropCount {
    fn drop(&mut self) {
        if let Some(shared) = &self.0 {
            shared.gates(self.1);
        }
    }
}

/// Devices that note when they close.
struct Closing {
    inner: Devices,
    _closed: DropCount,
}

impl fmt::Debug for Closing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl InputBackend for Closing {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        self.inner.button(button, state)
    }

    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError> {
        self.inner.key(key, state)
    }

    fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        self.inner.move_relative(dx, dy)
    }

    fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError> {
        self.inner.move_absolute(position)
    }

    fn scroll(&mut self, notches: i32) -> Result<(), InputError> {
        self.inner.scroll(notches)
    }
}

struct PerRunPlatform(PerRunShared);

impl Platform for PerRunPlatform {
    type Backend = Closing;

    const LIFETIME: InputLifetime = InputLifetime::PerRun;

    fn open_input(&mut self) -> OpenInput<Closing> {
        let (gate, decided) = oneshot::channel();
        self.0.gates(|g| g.open = Some(gate));
        let shared = self.0.clone();
        Box::pin(async move {
            let pending = DropCount::new(shared.clone(), |g| g.opens_dropped += 1);
            let decision = decided
                .await
                .unwrap_or_else(|_| Err(InputError::Portal("gate dropped".to_owned())));
            pending.disarm();
            decision?;
            let devices = shared.devices.bare_devices();
            Ok(Input::new(Closing {
                inner: devices,
                _closed: DropCount::new(shared, |g| g.closed += 1),
            }))
        })
    }

    fn input_lost(&self, _input: &Input<Closing>) -> InputLoss {
        let (loss, lost) = oneshot::channel();
        self.0.gates(|g| g.loss = Some(loss));
        Box::pin(async move {
            match lost.await {
                Ok(err) => err,
                Err(_) => pending().await,
            }
        })
    }

    fn read_hotkeys(&mut self, own: Option<&Input<Closing>>, send: HotkeySender) -> HotkeyReader {
        assert!(
            own.is_none(),
            "per-run devices are never shown to the reader"
        );
        FakePlatform(self.0.devices.clone()).read_hotkeys(None, send)
    }
}

async fn per_run_daemon(mode: Mode) -> (Daemon, PerRunShared) {
    let shared = PerRunShared::default();
    let config = Config {
        tray_start_delay: Duration::ZERO,
        ..config(mode)
    };
    let platform = PerRunPlatform(shared.clone());
    let (controller, links) = Controller::new(platform, None, config, None);
    let Links {
        send: inbox,
        view,
        released_for_sleep,
    } = links;
    let task = tokio::task::spawn_local(controller.run());
    let d = Daemon {
        inbox,
        released_for_sleep,
        view,
        shared: shared.devices.clone(),
        task,
    };
    d.send(Message::Session(None)).await;
    (d, shared)
}

#[tokio::test(start_paused = true)]
async fn a_start_waits_for_the_open_then_clicks_and_closes_the_devices_after() {
    local(async {
        let (d, gates) = per_run_daemon(Mode::HoldButton).await;
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Opening);
        assert!(d.shared.calls().is_empty());

        gates.finish_open(Ok(()));
        d.settle().await;
        assert_eq!(d.activity(), Activity::Clicking);
        assert_eq!(d.shared.calls(), [LEFT_DOWN]);

        d.send(Message::Stop).await;
        d.assert_idle_after(&[LEFT_DOWN, LEFT_UP]);
        assert_eq!(gates.gates(|g| g.closed), 1, "closed after the run");
        assert!(
            !gates.open_in_flight(),
            "nothing opens until the next start"
        );

        // The next start opens again.
        d.send(Message::Start).await;
        assert_eq!(d.activity(), Activity::Opening);
        gates.finish_open(Ok(()));
        d.settle().await;
        assert_eq!(d.shared.calls_to(1), [LEFT_DOWN]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn stop_quit_a_lock_and_the_hotkey_drop_a_pending_open_at_once() {
    for cancel in ["stop", "quit", "lock", "hotkey"] {
        local(async {
            let (d, gates) = per_run_daemon(Mode::HoldButton).await;
            d.send(Message::Start).await;
            assert!(gates.open_in_flight(), "{cancel}");
            let before = Instant::now();
            match cancel {
                "stop" => d.send(Message::Stop).await,
                "quit" => d.send(Message::Quit).await,
                "lock" => d.send(Message::LockRequested).await,
                _ => d.press_hotkey().await,
            }
            assert_eq!(Instant::now(), before, "{cancel}: no time passed");
            assert_eq!(gates.gates(|g| g.opens_dropped), 1, "{cancel}");
            assert!(!gates.open_in_flight(), "{cancel}");
            if cancel != "quit" {
                d.assert_idle_after(&[]);
            }
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn the_hotkey_reader_keeps_running_while_devices_open_and_close() {
    local(async {
        let (d, gates) = per_run_daemon(Mode::HoldButton).await;
        d.send(Message::Start).await;
        gates.finish_open(Ok(()));
        d.settle().await;
        // The hotkey stops the run: the reader is still the first one.
        d.press_hotkey().await;
        d.assert_idle_after(&[LEFT_DOWN, LEFT_UP]);
        assert_eq!(d.shared.steps(), [Step::ReaderStarted(None)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_failed_open_says_why_the_start_did_not_happen() {
    local(async {
        let (d, gates) = per_run_daemon(Mode::HoldButton).await;
        d.send(Message::Start).await;
        gates.finish_open(Err(InputError::NotAllowed));
        d.settle().await;
        d.assert_idle_after(&[]);
        assert!(
            d.notice().starts_with("Not started: not allowed"),
            "{}",
            d.notice()
        );
        // A per-run failure is told once, not kept as a problem.
        assert!(d.view().problems.is_empty(), "{:?}", d.view().problems);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_problem_found_at_start_shows_until_an_open_works() {
    local(async {
        let shared = PerRunShared::default();
        let config = Config {
            tray_start_delay: Duration::ZERO,
            ..config(Mode::HoldButton)
        };
        let (mut controller, links) =
            Controller::new(PerRunPlatform(shared.clone()), None, config, None);
        controller.report_input_problem(&InputError::PortalUnavailable("no portal".to_owned()));
        let task = tokio::task::spawn_local(controller.run());
        let d = Daemon {
            inbox: links.send,
            released_for_sleep: links.released_for_sleep,
            view: links.view,
            shared: shared.devices.clone(),
            task,
        };
        d.send(Message::Session(None)).await;
        let problems = d.view().problems;
        assert!(
            problems.iter().any(|p| p.contains("no portal")),
            "{problems:?}"
        );
        d.send(Message::Start).await;
        shared.finish_open(Ok(()));
        d.settle().await;
        assert_eq!(d.activity(), Activity::Clicking);
        assert!(d.view().problems.is_empty(), "{:?}", d.view().problems);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn losing_the_devices_stops_a_run_that_writes_nothing_at_once() {
    local(async {
        let (d, gates) = per_run_daemon(Mode::HoldButton).await;
        d.send(Message::Start).await;
        gates.finish_open(Ok(()));
        d.settle().await;
        assert_eq!(d.activity(), Activity::Clicking);

        let before = Instant::now();
        gates.lose(InputError::SessionLost("disconnected".to_owned()));
        d.settle().await;
        assert_eq!(Instant::now(), before, "no time passed");
        // The run released what it held (the write may fail on a lost
        // session; the desktop releases it then) and closed the devices.
        d.assert_idle_after(&[LEFT_DOWN, LEFT_UP]);
        assert!(
            d.notice()
                .starts_with("Clicking stopped: the desktop ended remote input"),
            "{}",
            d.notice()
        );
        assert_eq!(gates.gates(|g| g.closed), 1);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_loss_after_the_run_stopped_changes_nothing() {
    local(async {
        let (d, gates) = per_run_daemon(Mode::HoldButton).await;
        d.send(Message::Start).await;
        gates.finish_open(Ok(()));
        d.settle().await;
        d.send(Message::Stop).await;
        // The loss watch went with the run; nothing listens any more.
        assert!(gates.gates(|g| g.loss.as_ref().is_some_and(oneshot::Sender::is_closed)));
        d.assert_idle_after(&[LEFT_DOWN, LEFT_UP]);
        assert_eq!(d.notice(), "");
    })
    .await;
}
