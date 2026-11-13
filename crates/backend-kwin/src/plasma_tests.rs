//! Hand-run tests against the real KWin (R03 brief criterion 11, W01b).
//!
//! They need Plasma 6.8 or later on Wayland, and the daemon stopped
//! (`systemctl --user stop auto-clicker`): they take its bus name. They load
//! and unload the position script and check KWin's own `isScriptLoaded`; a
//! recording starts on the Wayland display's layout, so a position kept
//! shows KWin's layout and xdg-output's agree. From the repository root,
//! inside the session, one at a time:
//!
//! ```text
//! cargo test --locked -p auto-clicker-backend-kwin plasma -- --ignored --test-threads=1 --nocapture
//! ```

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use auto_clicker_backend_wayland::Wayland;
use auto_clicker_core::controller::LayoutSignal;
use auto_clicker_core::hotkeys::Bindings;
use auto_clicker_core::recording::{Change, Created, Ending, Point, Recorder, RecordingSlot};
use tokio::task::{JoinHandle, LocalSet};
use tokio::time::{sleep, timeout};
use zbus::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};

use crate::Kwin;
use crate::dir::ScriptDir;
use crate::script::Script;
use crate::scripting::{Bus, KWIN, Scripting};

/// The name the scripts call.
const BUS_NAME: &str = "io.github.tobylo.AutoClicker";
/// Generous: KWin answered within milliseconds in X01.
const WAIT: Duration = Duration::from_secs(5);

/// The event clock, from the tests' first use of it.
fn clock() -> Duration {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

/// A daemon's connection holding the name, as `instance::claim` makes it.
async fn claim() -> Connection {
    let bus = Connection::session().await.unwrap();
    let reply = bus
        .request_name_with_flags(BUS_NAME, RequestNameFlags::DoNotQueue.into())
        .await;
    assert!(
        matches!(reply, Ok(RequestNameReply::PrimaryOwner)),
        "stop the daemon first: {reply:?}"
    );
    bus
}

/// Which scripts KWin has loaded: an older version's layout script, the
/// position script.
async fn loaded(bus: &Connection) -> (bool, bool) {
    let probe = Bus(bus.clone());
    let (owner, _) = probe.follow_owner().await.unwrap();
    let owner = owner.expect("KWin runs");
    let layout = probe.is_loaded(&owner, Script::OldLayout).await.unwrap();
    let position = probe.is_loaded(&owner, Script::Position).await.unwrap();
    (layout, position)
}

/// Waits, within [`WAIT`], until KWin has loaded what `expected` says.
async fn until_loaded(bus: &Connection, expected: (bool, bool)) {
    let reached = async {
        while loaded(bus).await != expected {
            sleep(Duration::from_millis(50)).await;
        }
    };
    timeout(WAIT, reached)
        .await
        .unwrap_or_else(|_| panic!("KWin's scripts never became {expected:?}"));
}

/// The daemon's script directory.
fn script_dir() -> std::path::PathBuf {
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap();
    std::path::Path::new(&runtime).join("auto-clicker")
}

/// The script files left in the runtime directory.
fn files() -> Vec<String> {
    let dir = script_dir();
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".js"))
        .collect();
    names.sort();
    names
}

/// Serves and starts watching on `bus`, checking KWin against the
/// display; the watcher's task, once the start's sweep is done.
async fn start(bus: &Connection, slot: &RecordingSlot) -> (Kwin, JoinHandle<()>) {
    let display = Wayland::connect(tokio::time::Instant::now() + WAIT).await;
    let kwin = Kwin::serve(bus, clock, display.compositor()).await;
    let watcher = kwin.watch(
        slot.feed(),
        LayoutSignal::default(),
        LayoutSignal::default(),
    );
    let task = tokio::task::spawn_local(watcher);
    until_loaded(bus, (false, false)).await;
    sleep(Duration::from_millis(200)).await;
    (kwin, task)
}

/// A recording on the Wayland display's layout, as the controller starts
/// one.
async fn recorder() -> Recorder {
    let display = Wayland::connect(tokio::time::Instant::now() + WAIT).await;
    let layout = display.layout().expect("the display gives a layout");
    println!("the display's layout: {layout:?}");
    let created = Created::parse("2026-11-11T12:00:00.000+01:00").unwrap();
    Recorder::new(Bindings::default(), clock(), created, None).with_layout(layout)
}

/// The positions `recorder` kept.
fn positions(mut recorder: Recorder) -> Vec<Point> {
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

#[tokio::test]
#[ignore = "needs Plasma on Wayland, with the daemon stopped"]
async fn plasma_scripts_load_report_and_unload() {
    LocalSet::new()
        .run_until(async {
            let bus = claim().await;
            let slot = RecordingSlot::new();
            let (kwin, task) = start(&bus, &slot).await;
            assert_eq!(files(), Vec::<String>::new());

            slot.start(recorder().await);
            until_loaded(&bus, (false, true)).await;
            assert_eq!(files().len(), 1);
            // Time to move the mouse while the position script reports.
            sleep(Duration::from_secs(3)).await;
            let recorder = slot.take().unwrap();
            until_loaded(&bus, (false, false)).await;
            let kept = positions(recorder);
            println!("positions kept: {}", kept.len());
            assert!(!kept.is_empty(), "KWin's layout is not the display's");

            task.abort();
            kwin.exit().await;
            assert_eq!(loaded(&bus).await, (false, false));
            assert_eq!(files(), Vec::<String>::new());
        })
        .await;
}

#[tokio::test]
#[ignore = "needs Plasma on Wayland, with the daemon stopped"]
async fn plasma_a_crashed_daemons_scripts_are_unloaded_at_the_next_start() {
    LocalSet::new()
        .run_until(async {
            // A daemon that dies while recording: no exit runs.
            let bus = claim().await;
            let slot = RecordingSlot::new();
            let (kwin, task) = start(&bus, &slot).await;
            slot.start(recorder().await);
            until_loaded(&bus, (false, true)).await;
            task.abort();
            drop((kwin, slot));
            drop(bus);
            assert_eq!(files().len(), 1, "the crash left the script's file");

            // The next start, once it holds the name, cleans up.
            let bus = claim().await;
            let slot = RecordingSlot::new();
            let (kwin, task) = start(&bus, &slot).await;
            assert_eq!(files(), Vec::<String>::new());
            task.abort();
            kwin.exit().await;
            assert_eq!(loaded(&bus).await, (false, false));
            assert!(ScriptDir::from_env().is_ok());
        })
        .await;
}

#[tokio::test]
#[ignore = "needs Plasma on Wayland, with the daemon stopped"]
async fn plasma_an_upgrades_first_start_unloads_an_older_layout_script() {
    LocalSet::new()
        .run_until(async {
            // What a crashed daemon of a version before W01b left: its
            // layout script loaded, and its file.
            let bus = claim().await;
            let dir = ScriptDir::from_env().unwrap();
            let name = format!("{}-0123456789abcdef.js", Script::OldLayout.plugin());
            let path = dir.write(&name, "// an older layout script\n").unwrap();
            let scripting = zbus::Proxy::new(&bus, KWIN, "/Scripting", "org.kde.kwin.Scripting")
                .await
                .unwrap();
            let body = (path.to_str().unwrap(), Script::OldLayout.plugin());
            let id: i32 = scripting.call("loadScript", &body).await.unwrap();
            assert!(id >= 0, "KWin refused the older script");
            assert_eq!(loaded(&bus).await, (true, false));

            let slot = RecordingSlot::new();
            let (kwin, task) = start(&bus, &slot).await;
            assert_eq!(loaded(&bus).await, (false, false));
            assert!(!script_dir().join(&name).exists());
            task.abort();
            kwin.exit().await;
        })
        .await;
}

/// The plugin names of the foreign scripts.
const FOREIGN: [&str; 2] = ["auto-clicker-test-first", "auto-clicker-test-second"];

/// Where a foreign script's file goes: the user's own runtime directory,
/// which no other user can write to (not the shared temporary directory).
fn foreign_path(plugin: &str) -> std::path::PathBuf {
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap();
    std::path::Path::new(&runtime).join(format!("{plugin}.js"))
}

/// Another program's script, loaded (and so numbered) under `plugin` and
/// run, as a client of `/Scripting` does; its id.
async fn load_foreign(bus: &Connection, plugin: &str) -> i32 {
    let path = foreign_path(plugin);
    std::fs::write(&path, "// does nothing\n").unwrap();
    let scripting = zbus::Proxy::new(bus, KWIN, "/Scripting", "org.kde.kwin.Scripting")
        .await
        .unwrap();
    let id: i32 = scripting
        .call("loadScript", &(path.to_str().unwrap(), plugin))
        .await
        .unwrap();
    let script = zbus::Proxy::new(
        bus,
        KWIN,
        format!("/Scripting/Script{id}"),
        "org.kde.kwin.Script",
    )
    .await
    .unwrap();
    let () = script.call("run", &()).await.unwrap();
    id
}

async fn unload_foreign(bus: &Connection, plugin: &str) {
    let scripting = zbus::Proxy::new(bus, KWIN, "/Scripting", "org.kde.kwin.Scripting")
        .await
        .unwrap();
    let _: bool = scripting.call("unloadScript", &(plugin,)).await.unwrap();
    // Gone already when nothing was left.
    let _ = std::fs::remove_file(foreign_path(plugin));
}

#[tokio::test]
#[ignore = "needs Plasma on Wayland, with the daemon stopped"]
async fn plasma_a_script_given_the_id_of_a_running_one_still_runs() {
    LocalSet::new()
        .run_until(async {
            // KWin numbers a script by how many are loaded: with the first
            // of two unloaded, the next one gets the second one's id.
            let bus = claim().await;
            // What a failed run of this test left loaded goes first.
            for plugin in FOREIGN {
                unload_foreign(&bus, plugin).await;
            }
            let [first, second] = FOREIGN;
            load_foreign(&bus, first).await;
            let id = load_foreign(&bus, second).await;
            unload_foreign(&bus, first).await;

            // The position script is loaded under that id, and still reports.
            let slot = RecordingSlot::new();
            let (kwin, task) = start(&bus, &slot).await;
            slot.start(recorder().await);
            until_loaded(&bus, (false, true)).await;
            sleep(Duration::from_millis(500)).await;
            let kept = positions(slot.take().unwrap());
            println!(
                "the second script's id: {id}; positions kept: {}",
                kept.len()
            );
            assert!(!kept.is_empty());

            task.abort();
            kwin.exit().await;
            unload_foreign(&bus, second).await;
        })
        .await;
}

#[tokio::test]
#[ignore = "needs Plasma on Wayland"]
async fn plasma_kwins_owner_is_kwin_wayland_under_its_wrapper() {
    let probe = Bus(Connection::session().await.unwrap());
    let (owner, _) = probe.follow_owner().await.unwrap();
    let process = probe
        .owner_process(&owner.expect("KWin runs"))
        .await
        .unwrap();
    let name = |pid: u32| std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap();
    assert_eq!(name(process.pid).trim(), "kwin_wayland");
    // The wrapper listens on the display's socket (decision 0004); `comm`
    // holds 15 bytes of its name.
    let crate::display::Parent::Of(parent) = process.parent else {
        panic!("KWin's parent: {:?}", process.parent);
    };
    assert_eq!(name(parent).trim(), "kwin_wayland_wr");
}
