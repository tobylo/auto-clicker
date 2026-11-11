//! The watcher against a fake KWin: what is loaded and unloaded when, and
//! what reaches the recorder.

use std::cell::{Cell, RefCell};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use auto_clicker_core::hotkeys::Bindings;
use auto_clicker_core::recording::{Change, Created, Ending, Point, Recorder, RecordingSlot};
use futures_util::stream;
use tokio::task::{LocalSet, yield_now};
use tokio::time::timeout;

use super::*;
use crate::dir::ScriptDir;
use crate::receiver::{Call as Incoming, QUEUE, Receiver, Trust};

const KWIN: &str = ":1.7";
const SCREENS: &str = "0 0 2048 1280 1.25;2048 0 3440 1440 1";

thread_local! {
    /// The event clock, which only [`tick`] moves.
    static NOW: Cell<Duration> = const { Cell::new(Duration::from_secs(1)) };
}

fn clock() -> Duration {
    NOW.get()
}

/// A millisecond passes on the event clock: the recorder keeps one
/// position per millisecond.
fn tick() {
    NOW.set(NOW.get() + Duration::from_millis(1));
}

/// What the fake KWin was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Unload(&'static str),
    Load(&'static str),
    Run(&'static str),
}

use Call::{Load, Run, Unload};

const LAYOUT: &str = "auto-clicker-layout";
const POSITION: &str = "auto-clicker-position";

/// A started layout load and a started position load, as KWin sees them.
const LOAD_LAYOUT: [Call; 3] = [Unload(LAYOUT), Load(LAYOUT), Run(LAYOUT)];
const LOAD_POSITION: [Call; 3] = [Unload(POSITION), Load(POSITION), Run(POSITION)];

#[derive(Default)]
struct State {
    calls: Vec<Call>,
    owner: Option<String>,
    owners: Option<mpsc::UnboundedSender<Option<String>>>,
    /// Unloads that fail before the next one succeeds.
    failing_unloads: usize,
    /// Only this plugin's unloads fail, when set.
    failing_plugin: Option<&'static str>,
    /// Every call from now on never answers.
    hanging: bool,
    /// `start` never answers, once the script has run and reported.
    hanging_start: bool,
    /// KWin refuses to load this plugin.
    refused: Option<&'static str>,
    /// What a script reports as it runs; nothing when `None`.
    screens: Option<&'static str>,
    pointer: &'static str,
    /// The scripts loaded, by id: plugin and file.
    loaded: Vec<(&'static str, PathBuf)>,
    /// How many of `loaded` have been started.
    started: usize,
    /// The token each run script carried, by plugin, newest last.
    tokens: Vec<(&'static str, Token)>,
    receiver: Option<Rc<Receiver>>,
}

#[derive(Clone, Default)]
struct FakeKwin(Rc<RefCell<State>>);

impl FakeKwin {
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }

    async fn call(&self, call: Call) -> Result<(), CallError> {
        let hanging = self.with(|s| {
            s.calls.push(call);
            s.hanging
        });
        if hanging {
            pending::<()>().await;
        }
        Ok(())
    }

    /// The script reports as it runs, before KWin answers `start`.
    fn script_runs(&self, plugin: &'static str, path: &PathBuf) {
        let source = fs::read_to_string(path).unwrap();
        let token = source.split('"').nth(1).unwrap().to_owned();
        let (receiver, owner, screens, pointer) = self.with(|s| {
            s.tokens.push((plugin, Token::parse(&token).unwrap()));
            (
                s.receiver.clone().unwrap(),
                s.owner.clone(),
                s.screens,
                s.pointer,
            )
        });
        let Some(screens) = screens else { return };
        receiver.layout_from(Incoming {
            sender: owner.as_deref(),
            token: &token,
            text: screens,
        });
        if plugin == POSITION {
            receiver.position_from(Incoming {
                sender: owner.as_deref(),
                token: &token,
                text: pointer,
            });
        }
    }
}

impl Scripting for FakeKwin {
    async fn follow_owner(&self) -> Result<(Option<String>, Owners), CallError> {
        let (send, receive) = mpsc::unbounded_channel();
        let owner = self.with(|s| {
            s.owners = Some(send);
            s.owner.clone()
        });
        let owners = stream::unfold(receive, |mut receive| async move {
            receive.recv().await.map(|owner| (owner, receive))
        });
        Ok((owner, Box::pin(owners)))
    }

    async fn unload(&self, owner: &str, script: Script) -> Result<(), CallError> {
        assert_eq!(Some(owner), self.with(|s| s.owner.clone()).as_deref());
        self.call(Unload(script.plugin())).await?;
        self.with(|s| match s.failing_unloads {
            0 => Ok(()),
            _ if s
                .failing_plugin
                .is_some_and(|plugin| plugin != script.plugin()) =>
            {
                Ok(())
            }
            _ => {
                s.failing_unloads -= 1;
                Err(CallError("unload failed".to_owned()))
            }
        })
    }

    async fn load(&self, _owner: &str, path: &Path, script: Script) -> Result<i32, CallError> {
        self.call(Load(script.plugin())).await?;
        self.with(|s| {
            if s.refused == Some(script.plugin()) {
                return Ok(-1);
            }
            s.loaded.push((script.plugin(), path.to_path_buf()));
            Ok(i32::try_from(s.loaded.len() - 1).unwrap())
        })
    }

    /// Starts what was loaded since the last start, as KWin does.
    async fn start(&self, _owner: &str) -> Result<(), CallError> {
        let new = self.with(|s| {
            let new = s.loaded[s.started..].to_vec();
            s.started = s.loaded.len();
            new
        });
        for (plugin, path) in new {
            self.call(Run(plugin)).await?;
            self.script_runs(plugin, &path);
        }
        if self.with(|s| s.hanging_start) {
            pending::<()>().await;
        }
        Ok(())
    }
}

/// A fresh directory standing in for `$XDG_RUNTIME_DIR`.
struct Runtime(PathBuf);

impl Runtime {
    fn new() -> Runtime {
        let path = std::env::temp_dir().join(format!("auto-clicker-watch-{}", fastrand::u64(..)));
        fs::create_dir(&path).unwrap();
        Runtime(path)
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.0.join("auto-clicker"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Harness {
    kwin: FakeKwin,
    slot: RecordingSlot,
    session: Rc<Session<FakeKwin>>,
    receiver: Rc<Receiver>,
    changed: LayoutSignal,
    runtime: Runtime,
}

impl Harness {
    /// A fake KWin owned by `KWIN` (none when `running` is false) whose
    /// scripts report `SCREENS`, and a runtime directory holding `stale`.
    fn new(running: bool, stale: &[&str]) -> (Harness, Inbox) {
        let runtime = Runtime::new();
        let dir = ScriptDir::open(&runtime.0);
        for name in stale {
            fs::write(runtime.0.join("auto-clicker").join(name), "").unwrap();
        }
        let trust = Arc::new(Mutex::new(Trust::default()));
        let (queue, reports) = mpsc::channel(QUEUE);
        let layout_dropped = Arc::new(Notify::new());
        let receiver = Rc::new(Receiver {
            trust: Arc::clone(&trust),
            clock,
            queue,
            layout_dropped: Arc::clone(&layout_dropped),
        });
        let kwin = FakeKwin::default();
        kwin.with(|s| {
            s.owner = running.then(|| KWIN.to_owned());
            s.screens = Some(SCREENS);
            s.pointer = "100 200";
            s.receiver = Some(Rc::clone(&receiver));
        });
        let session = Rc::new(Session {
            scripting: kwin.clone(),
            dir,
            trust,
            layout: RefCell::default(),
            owner: RefCell::default(),
        });
        let harness = Harness {
            kwin,
            slot: RecordingSlot::new(),
            session,
            receiver,
            changed: LayoutSignal::default(),
            runtime,
        };
        let inbox = Inbox {
            reports,
            layout_dropped,
        };
        (harness, inbox)
    }

    fn watcher(&self) -> Watcher<FakeKwin> {
        Watcher::new(
            Rc::clone(&self.session),
            self.slot.feed(),
            self.changed.clone(),
        )
        .with_rng(fastrand::Rng::with_seed(11))
    }

    /// Starts the watcher on the local task set.
    async fn start(&self, inbox: Inbox) {
        tokio::task::spawn_local(self.watcher().run(inbox));
        settle().await;
    }

    fn calls(&self) -> Vec<Call> {
        self.kwin.with(|s| mem::take(&mut s.calls))
    }

    fn layout(&self) -> Option<Layout> {
        self.session.layout.borrow().clone()
    }

    fn newest_token(&self, plugin: &str) -> Token {
        self.kwin.with(|s| {
            let found = s.tokens.iter().rev().find(|(p, _)| *p == plugin);
            found.unwrap().1.clone()
        })
    }

    /// Starts a recording on the current layout.
    async fn record(&self) {
        let created = Created::parse("2026-11-11T12:00:00.000+01:00").unwrap();
        let mut recorder = Recorder::new(Bindings::default(), clock(), created, None);
        if let Some(layout) = self.layout() {
            recorder = recorder.with_layout(layout);
        }
        self.slot.start(recorder);
        settle().await;
    }

    /// Stops the recording, with a wheel turn so it is saved; its positions.
    async fn stop(&self) -> Vec<Point> {
        let recorder = self.slot.take();
        settle().await;
        let mut recorder = recorder.unwrap();
        tick();
        recorder.wheel(clock(), 1);
        let recording = recorder.finish(Ending::At(clock())).unwrap().unwrap();
        let positions = recording
            .events()
            .iter()
            .filter_map(|event| match event.change {
                Change::Position(point) => Some(point),
                _ => None,
            });
        positions.collect()
    }

    /// The pointer moved to `x`, as the position script of `token` reports.
    fn moved(&self, token: &Token, x: u32) {
        tick();
        let owner = self.kwin.with(|s| s.owner.clone());
        self.receiver.position_from(Incoming {
            sender: owner.as_deref(),
            token: token.as_str(),
            text: &format!("{x} 200"),
        });
    }

    async fn new_owner(&self, owner: Option<&str>) {
        self.kwin.with(|s| {
            s.owner = owner.map(str::to_owned);
            s.owners.as_ref().unwrap().send(s.owner.clone()).unwrap();
        });
        settle().await;
    }

    /// Whether the controller was told the layout changed since last asked.
    async fn told(&self) -> bool {
        timeout(Duration::ZERO, self.changed.fired()).await.is_ok()
    }
}

/// Lets every task run until it waits, without moving the paused clock.
async fn settle() {
    for _ in 0..64 {
        yield_now().await;
    }
}

/// Runs `watcher` as [`Watcher::run`] does, until it waits.
async fn drive(watcher: &mut Watcher<FakeKwin>, owners: &mut Owners, inbox: &mut Inbox) {
    loop {
        watcher.next_job();
        let Ok(wake) = timeout(Duration::ZERO, watcher.next_wake(owners, inbox)).await else {
            return;
        };
        watcher.handle(wake);
    }
}

/// Lets the paused clock run `wait` on, and every task until it waits.
async fn after(wait: Duration) {
    tokio::time::sleep(wait).await;
    settle().await;
}

const HOUR: Duration = Duration::from_secs(3600);

/// A layout load KWin refuses, as KWin sees it.
const REFUSED_LAYOUT: [Call; 2] = [Unload(LAYOUT), Load(LAYOUT)];

fn two_screens() -> Option<Layout> {
    crate::report::layout(SCREENS)
}

fn file_of(script: Script, token: &Token) -> String {
    script.file_name(token)
}

async fn local(test: impl Future<Output = ()>) {
    LocalSet::new().run_until(test).await;
}

#[tokio::test(start_paused = true)]
async fn the_start_sweeps_then_loads_the_layout_script() {
    local(async {
        let stale = ["auto-clicker-position-0123.js", "notes.txt"];
        let (h, inbox) = Harness::new(true, &stale);
        h.start(inbox).await;
        let swept = [Unload(LAYOUT), Unload(POSITION)];
        assert_eq!(h.calls(), [swept.as_slice(), &LOAD_LAYOUT].concat());
        assert_eq!(h.layout(), two_screens());
        assert!(h.told().await);
        let token = h.newest_token(LAYOUT);
        assert_eq!(
            h.runtime.files(),
            [file_of(Script::Layout, &token), "notes.txt".to_owned()]
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn without_kwin_nothing_is_loaded_and_nothing_is_called() {
    local(async {
        let (h, inbox) = Harness::new(false, &["auto-clicker-layout-0123.js"]);
        h.start(inbox).await;
        h.record().await;
        assert!(h.stop().await.is_empty());
        assert_eq!(h.calls(), []);
        assert_eq!(h.layout(), None);
        assert!(h.runtime.files().is_empty());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_loads_the_position_script_and_its_end_unloads_it() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.calls();
        h.record().await;
        assert_eq!(h.calls(), LOAD_POSITION);
        let token = h.newest_token(POSITION);
        h.moved(&token, 300);
        settle().await;
        // Every way a recording ends takes it out of the slot.
        let positions = h.stop().await;
        assert_eq!(
            positions,
            [Point { x: 100, y: 200 }, Point { x: 300, y: 200 }]
        );
        assert_eq!(h.calls(), [Unload(POSITION)]);
        let layout = h.newest_token(LAYOUT);
        assert_eq!(h.runtime.files(), [file_of(Script::Layout, &layout)]);
        // A report of the ended load reaches nothing.
        h.record().await;
        h.calls();
        h.moved(&token, 500);
        settle().await;
        assert_eq!(h.stop().await, [Point { x: 100, y: 200 }]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_stop_and_a_start_between_two_polls_give_the_new_recording_its_own_position() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.record().await;
        let first = h.newest_token(POSITION);
        h.calls();
        // The old load still reports while the controller stops one
        // recording and starts the next, before the watcher looks.
        let old = h.slot.take().unwrap();
        h.moved(&first, 700);
        h.kwin.with(|s| s.pointer = "900 200");
        h.record().await;
        drop(old);
        assert_eq!(h.calls(), LOAD_POSITION);
        assert_ne!(h.newest_token(POSITION), first);
        assert_eq!(h.stop().await, [Point { x: 900, y: 200 }]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn reports_reach_the_recorder_in_order_and_at_once_while_kwin_hangs() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.calls();
        // The position script runs and reports, but KWin never answers
        // `start`: the load stays in flight.
        h.kwin.with(|s| s.hanging_start = true);
        h.record().await;
        assert_eq!(h.calls(), LOAD_POSITION);
        let token = h.newest_token(POSITION);
        // Drained on the paused clock, while the call still hangs.
        for x in 1..=50 {
            h.moved(&token, x);
        }
        settle().await;
        let positions = h.stop().await;
        let xs: Vec<u32> = positions.iter().map(|point| point.x).collect();
        assert_eq!(
            xs,
            [[100].as_slice(), &(1..=50).collect::<Vec<_>>()].concat()
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_layout_the_position_script_sent_before_a_lost_one_is_not_taken() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.record().await;
        let token = h.newest_token(POSITION);
        // A lost layout report has the layout script loaded again, by a
        // call that never answers, so a new layout token is current.
        h.kwin.with(|s| s.hanging = true);
        h.receiver.layout_dropped.notify_one();
        settle().await;
        assert_eq!(h.calls().last(), Some(&Unload(LAYOUT)));
        // What the position script queued before the loss is older than
        // what was lost.
        h.receiver.layout_from(Incoming {
            sender: Some(KWIN),
            token: token.as_str(),
            text: SCREENS,
        });
        settle().await;
        assert_eq!(h.layout(), None);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_full_queue_drops_positions_and_the_recording_carries_on() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.record().await;
        let token = h.newest_token(POSITION);
        h.calls();
        // The daemon's thread does not get to the queue meanwhile.
        let full = u32::try_from(QUEUE).unwrap();
        for x in 1..=full + 10 {
            h.moved(&token, x);
        }
        settle().await;
        h.moved(&token, 5000);
        settle().await;
        let positions = h.stop().await;
        let xs: Vec<u32> = positions.iter().map(|point| point.x).collect();
        let expected = [[100].as_slice(), &(1..=full).collect::<Vec<_>>(), &[5000]].concat();
        assert_eq!(xs, expected);
        assert_eq!(
            h.calls(),
            [Unload(POSITION)],
            "a dropped position reloads nothing"
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_lost_layout_report_makes_the_layout_unknown_and_reloads_its_script() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        let token = h.newest_token(LAYOUT);
        h.calls();
        assert!(h.told().await);
        let full = u32::try_from(QUEUE).unwrap();
        for _ in 0..full {
            h.receiver.layout_from(Incoming {
                sender: Some(KWIN),
                token: token.as_str(),
                text: SCREENS,
            });
        }
        h.receiver.layout_from(Incoming {
            sender: Some(KWIN),
            token: token.as_str(),
            text: "0 0 2048 1280 1.25",
        });
        settle().await;
        assert_eq!(h.calls(), LOAD_LAYOUT);
        assert_eq!(h.layout(), two_screens());
        assert!(h.told().await, "never said it was unknown");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_layout_that_is_none_or_from_an_old_load_is_not_taken() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        assert!(h.told().await);
        let token = h.newest_token(LAYOUT);
        h.receiver.layout_from(Incoming {
            sender: Some(KWIN),
            token: token.as_str(),
            text: "10 0 5 5 1",
        });
        settle().await;
        assert_eq!((h.layout(), h.told().await), (None, true));
        h.receiver.layout_from(Incoming {
            sender: Some(KWIN),
            token: token.as_str(),
            text: SCREENS,
        });
        settle().await;
        assert_eq!((h.layout(), h.told().await), (two_screens(), true));
        // The same layout again changes nothing.
        h.receiver.layout_from(Incoming {
            sender: Some(KWIN),
            token: token.as_str(),
            text: SCREENS,
        });
        settle().await;
        assert!(!h.told().await);
        // A recording's position script reports the layout too.
        h.kwin.with(|s| s.screens = Some("0 0 2048 1280 1.25"));
        h.record().await;
        assert_eq!(h.layout(), crate::report::layout("0 0 2048 1280 1.25"));
        h.stop().await;
        let stopped = h.newest_token(POSITION);
        h.receiver.layout_from(Incoming {
            sender: Some(KWIN),
            token: stopped.as_str(),
            text: SCREENS,
        });
        settle().await;
        assert_eq!(h.layout(), crate::report::layout("0 0 2048 1280 1.25"));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_failed_unload_is_tried_again_before_the_next_load_and_at_exit() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.record().await;
        h.calls();
        h.kwin.with(|s| s.failing_unloads = 1);
        h.stop().await;
        assert_eq!(h.calls(), [Unload(POSITION)]);
        let failed = h.newest_token(POSITION);
        assert!(
            h.runtime
                .files()
                .contains(&file_of(Script::Position, &failed))
        );
        h.record().await;
        assert_eq!(h.calls(), LOAD_POSITION);
        assert!(
            !h.runtime
                .files()
                .contains(&file_of(Script::Position, &failed))
        );
        h.kwin.with(|s| s.failing_unloads = 1);
        h.stop().await;
        h.calls();
        h.session.exit().await;
        let mut exit = h.calls();
        exit.sort_by_key(|call| format!("{call:?}"));
        assert_eq!(exit, [Unload(LAYOUT), Unload(POSITION)]);
        assert!(h.runtime.files().is_empty());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_new_owner_of_kwin_gets_the_scripts_again() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.record().await;
        let old = h.newest_token(POSITION);
        h.calls();
        assert!(h.told().await);
        h.new_owner(None).await;
        // Nothing is known, nothing is called, and nothing old is heard.
        assert_eq!((h.layout(), h.told().await), (None, true));
        assert_eq!(h.calls(), []);
        h.moved(&old, 1);
        h.new_owner(Some(":1.9")).await;
        h.moved(&old, 2);
        assert_eq!(h.calls(), [LOAD_LAYOUT, LOAD_POSITION].concat());
        assert_eq!(h.layout(), two_screens());
        assert_ne!(h.newest_token(POSITION), old);
        let positions = h.stop().await;
        assert_eq!(positions.last(), Some(&Point { x: 100, y: 200 }));
        assert!(!positions.iter().any(|point| point.x < 100));
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_runtime_directory_open_to_others_loads_nothing() {
    local(async {
        let runtime = Runtime::new();
        let ours = runtime.0.join("auto-clicker");
        fs::create_dir(&ours).unwrap();
        fs::set_permissions(&ours, fs::Permissions::from_mode(0o777)).unwrap();
        let (h, inbox) = Harness::new(true, &[]);
        let session = Session {
            scripting: h.kwin.clone(),
            dir: ScriptDir::open(&runtime.0),
            trust: Arc::clone(&h.session.trust),
            layout: RefCell::default(),
            owner: RefCell::default(),
        };
        assert!(session.dir.is_err());
        let watcher = Watcher::new(Rc::new(session), h.slot.feed(), h.changed.clone());
        tokio::task::spawn_local(watcher.run(inbox));
        settle().await;
        h.record().await;
        let calls = h.calls();
        assert!(
            !calls.iter().any(|call| matches!(call, Load(_) | Run(_))),
            "{calls:?}"
        );
        assert!(fs::read_dir(&ours).unwrap().next().is_none());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_call_that_never_answers_ends_at_its_deadline() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.kwin.with(|s| s.hanging = true);
        h.start(inbox).await;
        assert_eq!(h.calls(), [Unload(LAYOUT), Unload(POSITION)]);
        tokio::time::sleep(crate::scripting::CALL_TIMEOUT).await;
        settle().await;
        // The sweep gave up; the layout load is the next call.
        assert_eq!(h.calls(), [Unload(LAYOUT)]);
        assert_eq!(h.layout(), None);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn without_a_layout_script_a_recordings_layout_is_not_kept() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.start(inbox).await;
        assert_eq!(h.layout(), None);
        // The position script reports the layout, but nothing would see it
        // change once the recording ends: it is not taken.
        h.record().await;
        assert_eq!(h.layout(), None);
        assert!(h.stop().await.is_empty());
        assert_eq!(h.layout(), None);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_lost_layout_report_ends_the_old_load_while_a_call_is_in_flight() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        let token = h.newest_token(LAYOUT);
        // A recording's position load hangs, so the layout script cannot be
        // loaded again yet.
        h.kwin.with(|s| s.hanging = true);
        h.record().await;
        let full = u32::try_from(QUEUE).unwrap();
        for _ in 0..=full {
            h.receiver.layout_from(Incoming {
                sender: Some(KWIN),
                token: token.as_str(),
                text: "0 0 2048 1280 1.25",
            });
        }
        settle().await;
        // What the old load queued before the loss says nothing about now.
        assert_eq!(h.layout(), None);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_exit_before_the_watcher_ran_unloads_what_a_crash_left() {
    local(async {
        // The daemon claimed its name and failed to start: the watcher never
        // saw KWin, which may still run a crashed daemon's scripts.
        let stale = ["auto-clicker-position-0123.js"];
        let (h, _inbox) = Harness::new(true, &stale);
        h.session.exit().await;
        let mut exit = h.calls();
        exit.sort_by_key(|call| format!("{call:?}"));
        assert_eq!(exit, [Unload(LAYOUT), Unload(POSITION)]);
        assert!(h.runtime.files().is_empty());

        // Without KWin, only the files go.
        let (h, _inbox) = Harness::new(false, &stale);
        h.session.exit().await;
        assert_eq!(h.calls(), []);
        assert!(h.runtime.files().is_empty());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_new_owner_does_not_wait_for_a_call_to_the_old_one() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.kwin.with(|s| s.hanging = true);
        h.start(inbox).await;
        h.calls();
        // The old KWin never answers the sweep; the new one answers at once.
        h.kwin.with(|s| s.hanging = false);
        h.new_owner(Some(":1.9")).await;
        assert_eq!(h.calls(), LOAD_LAYOUT);
        assert_eq!(h.layout(), two_screens());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_stop_ends_a_position_load_still_in_flight() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.calls();
        // KWin does not answer the load's first call; it answers what
        // follows.
        h.kwin.with(|s| s.hanging = true);
        h.record().await;
        h.kwin.with(|s| s.hanging = false);
        h.stop().await;
        // The load ends at the stop: its script is never loaded, and the
        // unload goes out at once.
        assert_eq!(h.calls(), [Unload(POSITION), Unload(POSITION)]);
        tokio::time::sleep(crate::scripting::CALL_TIMEOUT * 2).await;
        settle().await;
        assert_eq!(h.calls(), []);
        let layout = h.newest_token(LAYOUT);
        assert_eq!(h.runtime.files(), [file_of(Script::Layout, &layout)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_layout_load_that_fails_once_is_tried_again_after_its_back_off() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.start(inbox).await;
        assert_eq!(h.calls().last(), Some(&Load(LAYOUT)));
        assert_eq!(h.layout(), None);
        h.kwin.with(|s| s.refused = None);
        after(BACK_OFF[0] - Duration::from_millis(1)).await;
        assert_eq!(h.calls(), []);
        after(Duration::from_millis(1)).await;
        // No recording started: the back-off alone tried it again.
        assert_eq!(h.calls(), LOAD_LAYOUT);
        assert_eq!(h.layout(), two_screens());
        assert!(h.told().await);
        after(HOUR).await;
        assert_eq!(h.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_position_unload_that_fails_once_is_tried_again_after_its_back_off() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.record().await;
        let token = h.newest_token(POSITION);
        h.calls();
        h.kwin.with(|s| s.failing_unloads = 1);
        h.stop().await;
        assert_eq!(h.calls(), [Unload(POSITION)]);
        let file = file_of(Script::Position, &token);
        assert!(h.runtime.files().contains(&file));
        after(BACK_OFF[0]).await;
        assert_eq!(h.calls(), [Unload(POSITION)]);
        assert!(!h.runtime.files().contains(&file));
        after(HOUR).await;
        assert_eq!(h.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_kwin_that_keeps_failing_is_left_alone_until_its_owner_changes() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.start(inbox).await;
        h.calls();
        for wait in BACK_OFF {
            after(wait - Duration::from_millis(1)).await;
            assert_eq!(h.calls(), [], "tried before its back-off of {wait:?}");
            after(Duration::from_millis(1)).await;
            assert_eq!(h.calls(), REFUSED_LAYOUT);
        }
        after(HOUR).await;
        assert_eq!(h.calls(), []);
        // A new owner starts the tries afresh.
        h.new_owner(Some(":1.9")).await;
        assert_eq!(h.calls(), REFUSED_LAYOUT);
        after(BACK_OFF[0]).await;
        assert_eq!(h.calls(), REFUSED_LAYOUT);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_or_a_new_owner_during_a_back_off_acts_at_once() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.record().await;
        h.kwin.with(|s| s.failing_unloads = 1);
        h.stop().await;
        h.calls();
        // The recording's load replaces the unload waiting for its back-off.
        let begun = Instant::now();
        h.record().await;
        assert_eq!(h.calls(), LOAD_POSITION);
        h.stop().await;
        assert_eq!(h.calls(), [Unload(POSITION)]);
        assert!(begun.elapsed() < Duration::from_millis(100));
        after(HOUR).await;
        assert_eq!(h.calls(), []);

        // A layout load fails, and KWin restarts during its back-off.
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.receiver.layout_dropped.notify_one();
        settle().await;
        assert_eq!(h.calls(), REFUSED_LAYOUT);
        h.kwin.with(|s| s.refused = None);
        let begun = Instant::now();
        h.new_owner(Some(":1.9")).await;
        assert_eq!(h.calls(), LOAD_LAYOUT);
        assert!(begun.elapsed() < Duration::from_millis(100));
        after(HOUR).await;
        assert_eq!(h.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_during_a_layout_back_off_loads_first_and_the_layout_follows() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.start(inbox).await;
        h.calls();
        h.kwin.with(|s| s.refused = None);
        h.record().await;
        assert_eq!(h.calls(), LOAD_POSITION);
        after(BACK_OFF[0]).await;
        assert_eq!(h.calls(), LOAD_LAYOUT);
        assert_eq!(h.layout(), two_screens());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn with_nothing_failed_the_watcher_sets_no_timer() {
    local(async {
        let (h, mut inbox) = Harness::new(true, &[]);
        let mut watcher = h.watcher();
        let mut owners = watcher.follow_owner().await;
        drive(&mut watcher, &mut owners, &mut inbox).await;
        assert_eq!(h.calls().last(), Some(&Run(LAYOUT)));
        assert_eq!(watcher.retry_at, None);
        h.record().await;
        drive(&mut watcher, &mut owners, &mut inbox).await;
        h.stop().await;
        drive(&mut watcher, &mut owners, &mut inbox).await;
        assert_eq!(
            h.calls(),
            [LOAD_POSITION.as_slice(), &[Unload(POSITION)]].concat()
        );
        assert_eq!(watcher.retry_at, None);

        // A recording replaces an unload waiting for its back-off, and a
        // new owner a layout load: nothing is left to wait for.
        h.record().await;
        drive(&mut watcher, &mut owners, &mut inbox).await;
        h.kwin.with(|s| s.failing_unloads = 1);
        h.stop().await;
        drive(&mut watcher, &mut owners, &mut inbox).await;
        assert!(watcher.retry_at.is_some());
        h.record().await;
        drive(&mut watcher, &mut owners, &mut inbox).await;
        assert_eq!(watcher.retry_at, None);
        h.stop().await;
        drive(&mut watcher, &mut owners, &mut inbox).await;
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.receiver.layout_dropped.notify_one();
        drive(&mut watcher, &mut owners, &mut inbox).await;
        assert!(watcher.retry_at.is_some());
        h.kwin.with(|s| s.refused = None);
        h.new_owner(Some(":1.9")).await;
        drive(&mut watcher, &mut owners, &mut inbox).await;
        assert_eq!(watcher.retry_at, None);
        h.calls();

        // Nor once the tries are spent.
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.receiver.layout_dropped.notify_one();
        drive(&mut watcher, &mut owners, &mut inbox).await;
        assert!(watcher.retry_at.is_some());
        for wait in BACK_OFF {
            tokio::time::sleep(wait).await;
            drive(&mut watcher, &mut owners, &mut inbox).await;
        }
        assert_eq!(h.calls().len(), REFUSED_LAYOUT.len() * (BACK_OFF.len() + 1));
        assert_eq!(watcher.retry_at, None);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_script_directory_that_failed_its_check_is_not_tried_again() {
    local(async {
        let (h, mut inbox) = Harness::new(true, &[]);
        let session = Session {
            scripting: h.kwin.clone(),
            dir: Err("open to others".to_owned()),
            trust: Arc::clone(&h.session.trust),
            layout: RefCell::default(),
            owner: RefCell::default(),
        };
        let mut watcher = Watcher::new(Rc::new(session), h.slot.feed(), h.changed.clone());
        let mut owners = watcher.follow_owner().await;
        drive(&mut watcher, &mut owners, &mut inbox).await;
        assert_eq!(h.calls().last(), Some(&Unload(LAYOUT)));
        assert_eq!(watcher.retry_at, None);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_unload_that_fails_after_the_next_recording_started_leaves_its_load_due() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.record().await;
        // The unload hangs, and the next recording starts while it does.
        h.kwin.with(|s| s.hanging = true);
        h.stop().await;
        h.kwin.with(|s| s.hanging = false);
        h.record().await;
        h.calls();
        after(crate::scripting::CALL_TIMEOUT).await;
        assert_eq!(h.calls(), LOAD_POSITION);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_layout_load_given_up_is_tried_again_at_the_next_recording() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.start(inbox).await;
        for wait in BACK_OFF {
            after(wait).await;
        }
        after(HOUR).await;
        h.calls();
        h.kwin.with(|s| s.refused = None);
        h.record().await;
        assert_eq!(h.calls(), [LOAD_LAYOUT, LOAD_POSITION].concat());
        assert_eq!(h.layout(), two_screens());
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_crashed_daemons_position_script_that_fails_to_unload_at_start_is_tried_again() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.kwin.with(|s| {
            s.failing_unloads = 1;
            s.failing_plugin = Some(POSITION);
        });
        h.start(inbox).await;
        let swept = [Unload(LAYOUT), Unload(POSITION)];
        assert_eq!(h.calls(), [swept.as_slice(), &LOAD_LAYOUT].concat());
        after(BACK_OFF[0]).await;
        assert_eq!(h.calls(), [Unload(POSITION)]);
        after(HOUR).await;
        assert_eq!(h.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_second_failure_does_not_put_off_a_call_already_waiting() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.record().await;
        h.kwin.with(|s| {
            s.failing_unloads = 1;
            s.failing_plugin = Some(POSITION);
        });
        h.stop().await;
        h.calls();
        // A second later a layout load fails, with a longer back-off.
        after(Duration::from_secs(1)).await;
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.receiver.layout_dropped.notify_one();
        settle().await;
        assert_eq!(h.calls(), REFUSED_LAYOUT);
        h.kwin.with(|s| s.refused = None);
        after(BACK_OFF[0] - Duration::from_secs(1)).await;
        assert_eq!(
            h.calls(),
            [LOAD_LAYOUT.as_slice(), &[Unload(POSITION)]].concat()
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_position_script_loaded_during_a_layout_back_off_reports_no_layout() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.start(inbox).await;
        h.record().await;
        let token = h.newest_token(POSITION);
        // The layout load tried after the back-off never answers, so its
        // new token is current while no layout script runs.
        h.kwin.with(|s| {
            s.refused = None;
            s.hanging = true;
        });
        after(BACK_OFF[0]).await;
        assert_eq!(h.calls().last(), Some(&Unload(LAYOUT)));
        h.receiver.layout_from(Incoming {
            sender: Some(KWIN),
            token: token.as_str(),
            text: SCREENS,
        });
        settle().await;
        assert_eq!(h.layout(), None);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_recording_during_the_starts_layout_load_keeps_a_failed_sweep_unload() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        // The sweep's position unload fails, and the layout load that
        // follows stays in flight while a recording starts and stops.
        h.kwin.with(|s| {
            s.failing_unloads = 1;
            s.failing_plugin = Some(POSITION);
            s.hanging_start = true;
        });
        h.start(inbox).await;
        h.record().await;
        h.stop().await;
        h.kwin.with(|s| s.hanging_start = false);
        h.calls();
        after(crate::scripting::CALL_TIMEOUT).await;
        assert_eq!(h.calls(), [Unload(POSITION)]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_unload_whose_tries_the_layout_spent_is_tried_again_once_kwin_answers() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.start(inbox).await;
        h.record().await;
        h.kwin.with(|s| {
            s.failing_unloads = 2;
            s.failing_plugin = Some(POSITION);
        });
        h.stop().await;
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.receiver.layout_dropped.notify_one();
        settle().await;
        // Both are tried after the first back-off and fail again: the
        // unload's is the fourth failure, and it is given up on.
        after(BACK_OFF[0]).await;
        h.calls();
        h.kwin.with(|s| s.refused = None);
        after(BACK_OFF[2]).await;
        assert_eq!(
            h.calls(),
            [LOAD_LAYOUT.as_slice(), &[Unload(POSITION)]].concat()
        );
        after(HOUR).await;
        assert_eq!(h.calls(), []);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_report_lost_during_the_last_try_does_not_add_another() {
    local(async {
        let (h, inbox) = Harness::new(true, &[]);
        h.kwin.with(|s| s.refused = Some(LAYOUT));
        h.start(inbox).await;
        for wait in &BACK_OFF[..BACK_OFF.len() - 1] {
            after(*wait).await;
        }
        // The last try never answers, and a layout report is lost meanwhile.
        h.kwin.with(|s| s.hanging = true);
        after(BACK_OFF[BACK_OFF.len() - 1]).await;
        h.receiver.layout_dropped.notify_one();
        settle().await;
        h.calls();
        after(crate::scripting::CALL_TIMEOUT).await;
        assert_eq!(h.calls(), []);
    })
    .await;
}
