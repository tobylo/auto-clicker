//! X02 spike: the portal backend on Plasma 6.8, by hand, in a Plasma session.
//!
//! Throwaway code that answers the questions in docs/decisions/0002; it is not
//! built by the workspace and nothing in the product depends on it. See the
//! README next to Cargo.toml for how each result was produced.

mod capture;
mod detect;
mod eis;
mod log;
mod output;
mod remote;
mod shortcuts;

use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "usage: portal-backend-spike [--app-id ID] [--deadline SECS] COMMAND
commands:
  detect                              which input routes this session offers
  output TOKEN_FILE demo              buttons, keys, motion and scroll once each
  output TOKEN_FILE rate N MS         N left clicks, one every MS milliseconds
  output TOKEN_FILE hold WHAT HOW     hold WHAT (button|key), then end by HOW
                                      (kill|close|eis|stop|exit)
  output TOKEN_FILE combo             press and release Ctrl+F6 through libei
  output TOKEN_FILE locked            clicks and relative motion for a game
  output TOKEN_FILE park              pointer to the centre of the first output
  output TOKEN_FILE grant             start a session only (the permission)
  shortcuts list|bind|rebind|configure SECS
                                      list, register Ctrl+F6/F7/F8 (rebind: Ctrl+Shift+F6, Ctrl+F10, Ctrl+F11),
                                      configure: open the settings UI,
                                      then print activations
  capture SECS                        InputCapture: barrier at the top edge";

/// Options that apply to every command.
struct Global {
    app_id: Option<String>,
    deadline: Duration,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (global, command) = match parse_global(&args) {
        Ok(parsed) => parsed,
        Err(message) => return usage(&message),
    };
    // D11: the daemon runs on tokio's single-thread runtime; so does this.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => return fail(&format!("runtime: {err}")),
    };
    let local = tokio::task::LocalSet::new();
    let outcome = local.block_on(&runtime, run(global, command));
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => fail(&message),
    }
}

fn parse_global(args: &[String]) -> Result<(Global, &[String]), String> {
    let mut global = Global {
        app_id: None,
        deadline: Duration::from_secs(60),
    };
    let mut rest = args;
    loop {
        match rest {
            [flag, value, tail @ ..] if flag == "--app-id" => {
                global.app_id = Some(value.clone());
                rest = tail;
            }
            [flag, value, tail @ ..] if flag == "--deadline" => {
                global.deadline = Duration::from_secs(number(value)?);
                rest = tail;
            }
            _ => return Ok((global, rest)),
        }
    }
}

async fn run(global: Global, command: &[String]) -> Result<(), String> {
    if let Some(app_id) = &global.app_id {
        register(app_id).await?;
    }
    // Every run has a hard end: a dropped SSH session must not leave a
    // session open or a button held. Ending the process closes the EIS
    // socket, and the compositor releases what its devices held.
    tokio::time::timeout(global.deadline, dispatch(command))
        .await
        .unwrap_or_else(|_| Err(format!("deadline of {:?} reached", global.deadline)))
}

async fn dispatch(command: &[String]) -> Result<(), String> {
    match command {
        [name] if name == "detect" => detect::run().await,
        [name, token, rest @ ..] if name == "output" => output::run(token.as_ref(), rest).await,
        [name, mode, secs] if name == "shortcuts" => shortcuts::run(mode, number(secs)?).await,
        [name, secs] if name == "capture" => capture::run(number(secs)?).await,
        _ => Err(USAGE.to_owned()),
    }
}

async fn register(app_id: &str) -> Result<(), String> {
    let id = ashpd::AppID::try_from(app_id).map_err(|err| format!("app id {app_id}: {err}"))?;
    ashpd::register_host_app(id)
        .await
        .map_err(|err| format!("Registry.Register({app_id}): {err}"))?;
    log::line(&format!("registered host app id {app_id}"));
    Ok(())
}

/// A whole number from the command line.
pub fn number(text: &str) -> Result<u64, String> {
    text.parse()
        .map_err(|_| format!("not a whole number: {text}"))
}

fn usage(message: &str) -> ExitCode {
    eprintln!("{message}\n{USAGE}");
    ExitCode::from(2)
}

fn fail(message: &str) -> ExitCode {
    log::line(&format!("error: {message}"));
    ExitCode::FAILURE
}
