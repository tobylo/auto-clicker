//! Runs the clicker engine on the kernel backend with the settings of a
//! config file, for the click-rate and idle-cost measurements
//! (scripts/measure-clicker.py). Not part of the installed program.
//!
//! ```text
//! cargo run --release --locked -p auto-clicker-backend-kernel --example clicker -- CONFIG [START_DELAY_MS [RUN_FOR_MS]]
//! ```
//!
//! It prints `mouse <event node>` once the virtual devices exist, waits
//! START_DELAY_MS (default 1000) for the compositor to add them, prints
//! `start`, runs until the repeat count, Ctrl+C or SIGTERM, and prints how it
//! ended. With RUN_FOR_MS, it also stops by itself that long after it
//! started, so a run whose caller died cannot click on: the measurement
//! script always passes it. Leave the mouse and keyboard alone while it runs.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::process::ExitCode;
use std::time::Duration;

use auto_clicker_backend_kernel::{DeviceRole, KernelBackend};
use auto_clicker_core::clicker;
use auto_clicker_core::config::Config;
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::Instant;

fn main() -> ExitCode {
    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("clicker: {message}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "usage: clicker CONFIG [START_DELAY_MS [RUN_FOR_MS]]";

/// What the command line asks for.
struct Args {
    config: PathBuf,
    start_delay: Duration,
    /// How long after the start the run stops by itself, if at all.
    run_for: Option<Duration>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args_os().skip(1);
    let config = PathBuf::from(args.next().ok_or(USAGE)?);
    let start_delay = milliseconds(args.next(), "START_DELAY_MS")?;
    let run_for = milliseconds(args.next(), "RUN_FOR_MS")?;
    if args.next().is_some() {
        return Err(USAGE.to_owned());
    }
    Ok(Args {
        config,
        start_delay: start_delay.unwrap_or(Duration::from_secs(1)),
        run_for,
    })
}

fn milliseconds(arg: Option<OsString>, name: &str) -> Result<Option<Duration>, String> {
    arg.map(|ms| {
        ms.to_str()
            .and_then(|ms| ms.parse().ok())
            .map(Duration::from_millis)
            .ok_or(format!("{name} is a whole number of milliseconds"))
    })
    .transpose()
}

fn try_main() -> Result<(), String> {
    let args = parse_args()?;
    let loaded = Config::load(&args.config);
    if let Some(error) = loaded.error {
        return Err(error.to_string());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("could not start the runtime: {err}"))?;
    runtime.block_on(click(loaded.config, &args))
}

async fn click(config: Config, args: &Args) -> Result<(), String> {
    // A run time too large to add is an error, not "no limit".
    let deadline = args
        .run_for
        .map(|run_for| {
            Instant::now()
                .checked_add(run_for)
                .ok_or("RUN_FOR_MS is too large")
        })
        .transpose()?;
    let input = KernelBackend::open().map_err(|err| err.to_string())?;
    if let Some(node) = mouse_event_node(input.backend().devices()) {
        println!("mouse {}", node.display());
    }
    let mut terminate =
        signal(SignalKind::terminate()).map_err(|err| format!("could not watch SIGTERM: {err}"))?;
    let mut shutdown = pin!(async {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    });
    let mut stop = pin!(async {
        match deadline {
            Some(deadline) => tokio::time::sleep_until(deadline).await,
            None => std::future::pending().await,
        }
    });
    // A shutdown or the deadline during the start delay ends the program too.
    tokio::select! {
        () = &mut shutdown => {
            println!("ended ShutDown");
            return Ok(());
        }
        () = &mut stop => {
            println!("ended Stopped");
            return Ok(());
        }
        () = tokio::time::sleep(args.start_delay) => {}
    }
    println!("start");
    let signals = clicker::Signals { stop, shutdown };
    let end = clicker::run(input, &config.clicker, fastrand::Rng::new(), signals).await;
    let ended = end.result.map_err(|err| err.to_string())?;
    println!("ended {ended:?}");
    Ok(())
}

/// The `/dev/input/event*` node of the virtual mouse, for reading the
/// kernel's own timestamps of its clicks.
fn mouse_event_node(devices: &[auto_clicker_backend_kernel::DeviceIdentity]) -> Option<PathBuf> {
    let mouse = devices
        .iter()
        .find(|device| device.role == DeviceRole::Mouse)?;
    std::fs::read_dir(&mouse.syspath)
        .ok()?
        .map_while(Result::ok)
        .map(|entry| entry.file_name())
        .find(|name| name.to_string_lossy().starts_with("event"))
        .map(|name| Path::new("/dev/input").join(name))
}
