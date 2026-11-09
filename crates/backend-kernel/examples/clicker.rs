//! Runs the clicker engine on the kernel backend with the settings of a
//! config file, for the click-rate and idle-cost measurements
//! (scripts/measure-clicker.py). Not part of the installed program.
//!
//! ```text
//! cargo run --release --locked -p auto-clicker-backend-kernel --example clicker -- CONFIG [START_DELAY_MS]
//! ```
//!
//! It prints `mouse <event node>` once the virtual devices exist, waits
//! START_DELAY_MS (default 1000) for the compositor to add them, prints
//! `start`, runs until the repeat count, Ctrl+C or SIGTERM, and prints how it
//! ended. Leave the mouse and keyboard alone while it runs.

use std::path::{Path, PathBuf};
use std::pin::pin;
use std::process::ExitCode;
use std::time::Duration;

use auto_clicker_backend_kernel::{DeviceRole, KernelBackend};
use auto_clicker_core::clicker;
use auto_clicker_core::config::Config;
use tokio::signal::unix::{SignalKind, signal};

fn main() -> ExitCode {
    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("clicker: {message}");
            ExitCode::FAILURE
        }
    }
}

fn try_main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let config_path = PathBuf::from(
        args.next()
            .ok_or("usage: clicker CONFIG [START_DELAY_MS]")?,
    );
    let start_delay = match args.next() {
        Some(ms) => ms
            .to_str()
            .and_then(|ms| ms.parse().ok())
            .map(Duration::from_millis)
            .ok_or("START_DELAY_MS is a whole number of milliseconds")?,
        None => Duration::from_secs(1),
    };
    let loaded = Config::load(&config_path);
    if let Some(error) = loaded.error {
        return Err(error.to_string());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("could not start the runtime: {err}"))?;
    runtime.block_on(click(loaded.config, start_delay))
}

async fn click(config: Config, start_delay: Duration) -> Result<(), String> {
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
    // A shutdown during the start delay ends the program at once too.
    tokio::select! {
        () = &mut shutdown => {
            println!("ended ShutDown");
            return Ok(());
        }
        () = tokio::time::sleep(start_delay) => {}
    }
    println!("start");
    let signals = clicker::Signals {
        stop: std::future::pending(),
        shutdown,
    };
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
