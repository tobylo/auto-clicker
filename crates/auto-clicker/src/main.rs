//! The auto-clicker daemon: the tray icon, hotkeys and the clicker, run by one
//! controller on a single thread.

mod detect;
mod instance;
mod platform;
mod session;
mod tray;

use std::process::ExitCode;
use std::time::Duration;

use auto_clicker_backend_portal::{PortalOutput, token_path};
use auto_clicker_core::config::{Config, ConfigFile, config_path};
use auto_clicker_core::controller::{Controller, Links, Message, Platform};
use auto_clicker_core::input::{Input, InputError};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::timeout;

use detect::{Detected, Output, detect};
use instance::ClaimError;
use platform::{Hotkeys, Kernel, Portal};

/// Another daemon is running in this session. The user service does not
/// restart on it.
const EXIT_ALREADY_RUNNING: u8 = 3;
/// How long the session bus may take to answer at start: claiming the name
/// and registering the tray icon.
const BUS_TIMEOUT: Duration = Duration::from_secs(10);

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
    serve(Kernel { hotkeys }, start, name).await
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
    config_error: Option<String>,
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
    let Links {
        send,
        view: views,
        released_for_sleep,
    } = links;
    forward_signals(send.clone())
        .map_err(|err| fail(&format!("could not watch for signals: {err}")))?;
    match timeout(BUS_TIMEOUT, tray::show(views, send.clone())).await {
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
fn load_config() -> (Config, Option<String>, ConfigFile) {
    let Some(path) = config_path() else {
        let error = "no config directory: neither XDG_CONFIG_HOME nor HOME is set";
        eprintln!("auto-clicker: {error}; using the defaults");
        return (
            Config::default(),
            Some(error.to_owned()),
            ConfigFile::none(),
        );
    };
    let loaded = Config::load(&path);
    let error = loaded.error.map(|err| err.to_string());
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
