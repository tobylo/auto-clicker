//! The auto-clicker daemon: the tray icon, hotkeys and the clicker, run by one
//! controller on a single thread.

mod detect;
mod instance;
mod platform;
mod session;
mod tray;
mod tray_icon;
mod tray_text;
mod window;

use std::process::ExitCode;
use std::time::Duration;

use auto_clicker_backend_hyprland::Hyprland;
use auto_clicker_backend_kernel::monotonic_now;
use auto_clicker_backend_kwin::Kwin;
use auto_clicker_backend_portal::{PortalOutput, token_path};
use auto_clicker_backend_wayland::Wayland;
use auto_clicker_core::config::{Config, ConfigFile, config_path};
use auto_clicker_core::controller::{Controller, Links, Message, Platform};
use auto_clicker_core::input::{Input, InputError};
use auto_clicker_core::message::ConfigProblem;
use auto_clicker_core::recording::CurrentFile;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::timeout;

use detect::{Detected, Output, detect};
use instance::ClaimError;
use platform::{Desktop, Hotkeys, Kernel, Portal};

/// Another daemon is running in this session. The user service does not
/// restart on it.
const EXIT_ALREADY_RUNNING: u8 = 3;
/// How long the session bus may take to answer at start: claiming the name
/// and registering the tray icon.
const BUS_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the Wayland display may take at start: connecting, and the two
/// round trips that read the first layout.
const DISPLAY_TIMEOUT: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    match runtime {
        Ok(runtime) => runtime.block_on(daemon()),
        Err(err) => fail(&format!("could not start the runtime: {err}")),
    }
}

/// Runs the daemon until Quit, SIGTERM or SIGINT.
async fn daemon() -> ExitCode {
    // Claimed first: a second daemon reads no config, opens no device and
    // reads no keyboard.
    let name = match claim_name().await {
        Ok(name) => name,
        Err(status) => return status,
    };
    // The layout's and the pointer's sources are picked once, with the name
    // claimed. Only now may anything clean up after an earlier daemon: a
    // second one never touches its scripts. However this one ends from here
    // on, it unloads them.
    let wayland = connect_wayland().await;
    let kwin = serve_kwin(&name, &wayland).await;
    let hyprland = Hyprland::find(&wayland, monotonic_now).await;
    let desktop = Desktop {
        wayland,
        kwin: kwin.clone(),
        hyprland,
    };
    let status = run(name, desktop).await;
    kwin.exit().await;
    status
}

/// Picks the routes and runs the controller on them until it ends.
async fn run(name: instance::Claim, desktop: Desktop) -> ExitCode {
    let (config, config_error, file) = load_config();
    let portal = PortalOutput::new(token_path());
    // Whatever detection finds, the daemon runs: a job without a route says
    // why in the tray, and its next start tries again.
    let Detected { output, hotkeys } = detect(config.input, config.hotkeys, &portal).await;
    let hotkeys = Hotkeys { reader: hotkeys };
    let (input, problem) = match output {
        Output::Kernel(Ok(input)) => (Some(input), None),
        Output::Kernel(Err(problem)) => (None, Some(problem)),
        Output::Portal(problem) => {
            let start = Start {
                input: None,
                problem: reported(problem),
                config,
                config_error,
                file,
            };
            return serve(
                Portal {
                    output: portal,
                    hotkeys,
                    desktop,
                },
                start,
                name,
            )
            .await;
        }
    };
    let start = Start {
        input,
        problem: reported(problem),
        config,
        config_error,
        file,
    };
    let kernel = Kernel { hotkeys, desktop };
    serve(kernel, start, name).await
}

/// `problem`, written to the journal too.
fn reported(problem: Option<InputError>) -> Option<InputError> {
    if let Some(problem) = &problem {
        eprintln!("auto-clicker: {problem}");
    }
    problem
}

/// What the controller starts with.
struct Start<P: Platform> {
    /// The devices, when they are kept between runs.
    input: Option<Input<P::Backend>>,
    /// What a check at start found wrong with the devices.
    problem: Option<InputError>,
    config: Config,
    config_error: Option<ConfigProblem>,
    /// The config file as read, where the settings window saves.
    file: ConfigFile,
}

/// Runs a controller on `platform` until it ends.
async fn serve<P: Platform>(platform: P, start: Start<P>, name: instance::Claim) -> ExitCode {
    match connect(platform, start, name).await {
        Ok(controller) => {
            controller.run().await;
            ExitCode::SUCCESS
        }
        Err(status) => status,
    }
}

/// The daemon's Wayland display, and the first layout from it; none when it
/// offers no xdg-output or does not answer in time. After a KWin restart
/// its watcher connects again, with no deadline (W01 brief, W5).
async fn connect_wayland() -> Wayland {
    Wayland::connect(tokio::time::Instant::now() + DISPLAY_TIMEOUT).await
}

/// Receives from KWin on the claimed name; without it, recordings get no
/// positions. Nothing is loaded until the controller runs, and nothing into
/// a KWin that is not `wayland`'s compositor.
async fn serve_kwin(name: &instance::Claim, wayland: &Wayland) -> Kwin {
    let serve = Kwin::serve(name.connection(), monotonic_now, wayland.compositor());
    match timeout(BUS_TIMEOUT, serve).await {
        Ok(kwin) => kwin,
        Err(_) => {
            eprintln!("auto-clicker: the session bus did not answer; recordings get no positions");
            Kwin::none()
        }
    }
}

/// Everything before the controller runs; the exit status when it fails.
async fn connect<P: Platform>(
    platform: P,
    start: Start<P>,
    name: instance::Claim,
) -> Result<Controller<P>, ExitCode> {
    let (mut controller, links) =
        Controller::new(platform, start.input, start.config, start.config_error);
    if let Some(problem) = &start.problem {
        controller.report_input_problem(problem);
    }
    controller.keep_config_file(start.file);
    controller.keep_current_file(CurrentFile::of_environment());
    let Links {
        send,
        view: views,
        released_for_sleep,
    } = links;
    forward_signals(send.clone())
        .map_err(|err| fail(&format!("could not watch for signals: {err}")))?;
    let settings = window::keep(controller.window_opener());
    match timeout(BUS_TIMEOUT, tray::show(views, send.clone(), settings)).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(fail(&format!("tray icon: {err}"))),
        Err(_) => return Err(fail("tray icon: the session bus did not answer")),
    }
    tokio::spawn(session::watch(send.clone(), released_for_sleep));
    tokio::spawn(async move {
        name.lost().await;
        eprintln!("auto-clicker: lost the session bus name; quitting");
        let _ = send.send(Message::Quit);
    });
    Ok(controller)
}

/// Claims the single-instance name on the session bus.
async fn claim_name() -> Result<instance::Claim, ExitCode> {
    match timeout(BUS_TIMEOUT, instance::claim()).await {
        Ok(Ok(bus)) => Ok(bus),
        Ok(Err(ClaimError::Taken)) => {
            eprintln!("auto-clicker is already running");
            Err(ExitCode::from(EXIT_ALREADY_RUNNING))
        }
        Ok(Err(ClaimError::Transport(why))) => Err(fail(&why)),
        Ok(Err(ClaimError::Bus(err))) => Err(fail(&format!("session bus: {err}"))),
        Err(_) => Err(fail("the session bus did not answer")),
    }
}

/// The config, its error as the tray shows it, and the file as read.
fn load_config() -> (Config, Option<ConfigProblem>, ConfigFile) {
    let Some(path) = config_path() else {
        let error = ConfigProblem::NoDirectory;
        eprintln!("auto-clicker: {error}; using the defaults");
        return (Config::default(), Some(error), ConfigFile::none());
    };
    let loaded = Config::load(&path);
    let error = loaded.error.as_ref().map(ConfigProblem::from);
    if let Some(error) = &error {
        eprintln!("auto-clicker: {error}; using the defaults");
    }
    (loaded.config, error, loaded.file)
}

/// SIGTERM and SIGINT ask the controller to quit, which releases everything.
fn forward_signals(send: mpsc::UnboundedSender<Message>) -> std::io::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    tokio::spawn(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
        let _ = send.send(Message::Quit);
    });
    Ok(())
}

fn fail(message: &str) -> ExitCode {
    eprintln!("auto-clicker: {message}");
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_found_at_start_is_kept_for_the_tray() {
        let problem = InputError::Portal("no answer".to_owned());
        assert!(matches!(
            reported(Some(problem)),
            Some(InputError::Portal(why)) if why == "no answer"
        ));
        assert!(reported(None).is_none());
    }
}
