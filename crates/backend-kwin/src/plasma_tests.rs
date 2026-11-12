//! Hand-run tests against the real KWin (brief criterion 11).
//!
//! They need Plasma 6.8 or later on Wayland, and the daemon stopped
//! (`systemctl --user stop auto-clicker`): they take its bus name. They load
//! and unload the two scripts and check KWin's own `isScriptLoaded`. Move the
//! mouse a little while the second test's recording runs. From the
//! repository root, inside the session, one at a time:
//!
//! ```text
//! cargo test --locked -p auto-clicker-backend-kwin plasma -- --ignored --test-threads=1 --nocapture
//! ```

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use auto_clicker_core::controller::LayoutSignal;
use auto_clicker_core::hotkeys::Bindings;
use auto_clicker_core::recording::{Created, Recorder, RecordingSlot};
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

/// Which scripts KWin has loaded: layout, position.
async fn loaded(bus: &Connection) -> (bool, bool) {
    let probe = Bus(bus.clone());
    let (owner, _) = probe.follow_owner().await.unwrap();
    let owner = owner.expect("KWin runs");
    let layout = probe.is_loaded(&owner, Script::Layout).await.unwrap();
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

/// The script files left in the runtime directory.
fn files() -> Vec<String> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap();
    let dir = std::path::Path::new(&runtime).join("auto-clicker");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".js"))
        .collect();
    names.sort();
    names
}

/// Serves and starts watching on `bus`; the watcher's task.
async fn start(bus: &Connection, slot: &RecordingSlot) -> (Kwin, LayoutSignal, JoinHandle<()>) {
    let kwin = Kwin::serve(bus, clock, None).await;
    let changed = LayoutSignal::default();
    let watcher = kwin.watch(slot.feed(), changed.clone());
    let task = tokio::task::spawn_local(watcher);
    timeout(WAIT, changed.fired())
        .await
        .expect("the layout script reports");
    (kwin, changed, task)
}

fn recorder(kwin: &Kwin) -> Recorder {
    let created = Created::parse("2026-11-11T12:00:00.000+01:00").unwrap();
    let recorder = Recorder::new(Bindings::default(), clock(), created, None);
    recorder.with_layout(kwin.layout().expect("the layout is known"))
}

#[tokio::test]
#[ignore = "needs Plasma on Wayland, with the daemon stopped"]
async fn plasma_scripts_load_report_and_unload() {
    LocalSet::new()
        .run_until(async {
            let bus = claim().await;
            let slot = RecordingSlot::new();
            let (kwin, _changed, task) = start(&bus, &slot).await;
            println!("layout: {:?}", kwin.layout());
            assert_eq!(loaded(&bus).await, (true, false));
            assert_eq!(files().len(), 1);

            slot.start(recorder(&kwin));
            until_loaded(&bus, (true, true)).await;
            // Time to move the mouse while the position script reports.
            sleep(Duration::from_secs(3)).await;
            let recorder = slot.take().unwrap();
            until_loaded(&bus, (true, false)).await;
            drop(recorder);

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
            let (kwin, _changed, task) = start(&bus, &slot).await;
            slot.start(recorder(&kwin));
            until_loaded(&bus, (true, true)).await;
            task.abort();
            drop((kwin, slot));
            drop(bus);
            assert_eq!(files().len(), 2, "the crash left both files");

            // The next start, once it holds the name, cleans up.
            let bus = claim().await;
            let slot = RecordingSlot::new();
            let (kwin, _changed, task) = start(&bus, &slot).await;
            assert_eq!(loaded(&bus).await, (true, false));
            assert_eq!(files().len(), 1);
            task.abort();
            kwin.exit().await;
            assert_eq!(loaded(&bus).await, (false, false));
            assert!(ScriptDir::from_env().is_ok());
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

            // The layout script is loaded under that id, and still reports.
            let slot = RecordingSlot::new();
            let (kwin, _changed, task) = start(&bus, &slot).await;
            println!("the second script's id: {id}; layout: {:?}", kwin.layout());
            assert!(kwin.layout().is_some());

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
