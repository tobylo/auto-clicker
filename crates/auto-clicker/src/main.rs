//! The auto-clicker daemon: the tray icon, hotkeys and the clicker, run by one
//! controller on a single thread.

mod instance;
mod platform;
mod session;
mod tray;

use std::process::ExitCode;
use std::time::Duration;

use auto_clicker_backend_kernel::KernelBackend;
use auto_clicker_backend_portal::{PortalOutput, token_path};
use auto_clicker_core::config::{Config, ConfigFile, OutputRoute, config_path};
use auto_clicker_core::controller::{Controller, Links, Message, Platform};
use auto_clicker_core::input::{Input, InputError};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::timeout;

use instance::ClaimError;
use platform::{Hotkeys, Kernel, Portal};

/// Another daemon is running in this session. The user service does not
/// restart on it.
const EXIT_ALREADY_RUNNING: u8 = 3;
/// `/dev/uinput` cannot be used: no permission, or the module is missing.
/// Restarting cannot fix either, so the user service does not restart on it
/// (`EX_NOPERM`).
const EXIT_NO_UINPUT: u8 = 77;
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
    let hotkeys = Hotkeys {
        route: config.input.hotkeys,
    };
    match config.input.output {
        OutputRoute::Kernel => {
            let input = match KernelBackend::open() {
                Ok(input) => input,
                Err(err) => {
                    eprintln!("auto-clicker: {err}");
                    return ExitCode::from(open_failure(&err));
                }
            };
            let start = Start {
                input: Some(input),
                problem: None,
                config,
                config_error,
                file,
            };
            serve(Kernel { hotkeys }, start, name).await
        }
        OutputRoute::Portal => {
            let output = PortalOutput::new(token_path());
            // No dialog here: a session opens at each start. The check
            // registers the app id and shows what is missing, if anything.
            let problem = output.probe().await.err();
            if let Some(problem) = &problem {
                eprintln!("auto-clicker: {problem}");
            }
            let start = Start {
                input: None,
                problem,
                config,
                config_error,
                file,
            };
            serve(Portal { output, hotkeys }, start, name).await
        }
    }
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

/// The exit status for kernel devices that could not be opened at start.
fn open_failure(err: &InputError) -> u8 {
    match err {
        InputError::NoPermission(_) | InputError::UinputMissing(_) => EXIT_NO_UINPUT,
        _ => 1,
    }
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
    use std::io;

    use super::*;

    #[test]
    fn permanent_uinput_failures_get_their_own_status() {
        let err = |kind: io::ErrorKind| io::Error::from(kind);
        let denied = InputError::NoPermission(err(io::ErrorKind::PermissionDenied));
        let missing = InputError::UinputMissing(err(io::ErrorKind::NotFound));
        let create = InputError::CreateDevice(err(io::ErrorKind::Other));
        assert_eq!(open_failure(&denied), EXIT_NO_UINPUT);
        assert_eq!(open_failure(&missing), EXIT_NO_UINPUT);
        assert_eq!(open_failure(&create), 1);
    }
}
