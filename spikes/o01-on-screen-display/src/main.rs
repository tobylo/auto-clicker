//! O01a probe (decision record 0005).
//!
//!   o01-probe globals             list the display's globals and versions
//!   o01-probe outputs             each output's name, logical size and place
//!   o01-probe draw W H SCALE [SECONDS]
//!                                 times drawing the countdown ("88", or the
//!                                 label for SECONDS: 5940 gives "99m") for a
//!                                 screen W x H logical px at SCALE
//!   o01-probe show                the display; commands on stdin, one a line:
//!                                 `number N`, `badge`, `both N`, `pattern`,
//!                                 `none`, `stats`, `quit`
//!   o01-probe window locked|free [OUTPUT]
//!                                 a fullscreen test window (on OUTPUT, by
//!                                 name), holding a persistent pointer lock
//!                                 or not, printing what it hears; quits on
//!                                 stdin's end
//!
//! Every line printed starts with seconds on CLOCK_MONOTONIC.

mod dispatch;
mod display;
mod display_events;
mod glyphs;
mod paint;
mod pump;
mod say;
mod shm;
mod window;
mod window_events;

use std::process::ExitCode;

use wayland_client::globals::registry_queue_init;
use wayland_client::{Connection, QueueHandle};

use paint::Picture;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("globals") => globals(),
        Some("show") => show(false),
        Some("outputs") => show(true),
        Some("draw") => draw(&args[1..]),
        Some("window") => window(
            args.get(1).map(String::as_str) != Some("free"),
            args.get(2).cloned(),
        ),
        _ => Err(
            "usage: o01-probe globals | outputs | show | window locked|free [OUTPUT]".to_owned(),
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            say!("ended: {error}");
            ExitCode::FAILURE
        }
    }
}

fn connect() -> Result<Connection, String> {
    Connection::connect_to_env().map_err(|error| format!("connect: {error}"))
}

fn globals() -> Result<(), String> {
    let connection = connect()?;
    let (globals, _) =
        registry_queue_init::<window::State>(&connection).map_err(|error| format!("{error}"))?;
    for global in globals.contents().clone_list() {
        println!("{} v{}", global.interface, global.version);
    }
    Ok(())
}

/// Reads one command: what should show, or `None` for a command that is
/// not about that.
fn command(line: &str) -> Option<Vec<Picture>> {
    let mut words = line.split_whitespace();
    let verb = words.next()?;
    let seconds = words.next().and_then(|word| word.parse().ok()).unwrap_or(5);
    Some(match verb {
        "number" => vec![Picture::Number(seconds)],
        "badge" => vec![Picture::Badge],
        "both" => vec![Picture::Number(seconds), Picture::Badge],
        "pattern" => vec![Picture::Pattern],
        "none" => Vec::new(),
        _ => return None,
    })
}

fn show(list: bool) -> Result<(), String> {
    let connection = connect()?;
    let (globals, mut queue) =
        registry_queue_init::<display::State>(&connection).map_err(|error| format!("{error}"))?;
    let qh = queue.handle();
    let mut state = display::State::new(display::bind(&globals, &qh)?);
    let outputs = globals
        .contents()
        .clone_list()
        .into_iter()
        .filter(|global| global.interface == "wl_output");
    outputs.for_each(|global| {
        display_events::add_screen(&mut state, globals.registry(), &global, &qh)
    });
    if list {
        queue
            .roundtrip(&mut state)
            .map_err(|error| format!("{error}"))?;
        return queue
            .roundtrip(&mut state)
            .map(drop)
            .map_err(|error| format!("{error}"));
    }
    say!("display ready; {}", say::stats());
    let mut input = pump::Input::new();
    while !input.closed() {
        let lines = pump::wait(&mut queue, &mut state, &mut input, None)?;
        if obey(&mut state, &lines, &qh) == Obeyed::Quit {
            return Ok(());
        }
    }
    Ok(())
}

#[derive(PartialEq)]
enum Obeyed {
    All,
    Quit,
}

/// Carries out the display's commands in order, up to a `quit`.
fn obey(state: &mut display::State, lines: &[String], qh: &QueueHandle<display::State>) -> Obeyed {
    for line in lines {
        match (line.as_str(), command(line)) {
            (_, Some(wanted)) => state.set(wanted, qh),
            ("stats", None) => say!("{}", say::stats()),
            ("quit", None) => return Obeyed::Quit,
            (other, None) => say!("unknown command {other:?}"),
        }
    }
    Obeyed::All
}

fn window(lock: bool, output: Option<String>) -> Result<(), String> {
    let connection = connect()?;
    let (globals, mut queue) =
        registry_queue_init::<window::State>(&connection).map_err(|error| format!("{error}"))?;
    let qh = queue.handle();
    let output = match output {
        Some(name) => Some(window::output(&connection, &globals, &name)?),
        None => None,
    };
    let mut state = window::open(&globals, &qh, lock, output.as_ref())?;
    let mut input = pump::Input::new();
    while !input.closed() && !state.closed {
        let lines = pump::wait(&mut queue, &mut state, &mut input, Some(say::now() + 0.1))?;
        state.report();
        if lines.iter().any(|line| line == "quit") {
            break;
        }
    }
    state.report();
    say!("window closed; {}", say::stats());
    Ok(())
}

/// Times one draw of the widest countdown, "88", for a screen of the given
/// logical size and scale, as the display would draw it.
fn draw(args: &[String]) -> Result<(), String> {
    let number = |index: usize| {
        args.get(index)
            .and_then(|word| word.parse::<f64>().ok())
            .ok_or("usage: draw W H SCALE [SECONDS]")
    };
    let (width, height, scale) = (number(0)? as u32, number(1)? as u32, number(2)?);
    // "88" by default; 5940 s gives "99m", the widest label.
    let picture = Picture::Number(number(3).map_or(88, |seconds| seconds as u64));
    let logical = paint::logical_size(picture, (width, height));
    let buffer = (
        (f64::from(logical.0) * scale).round() as u32,
        (f64::from(logical.1) * scale).round() as u32,
    );
    let mut times = Vec::new();
    for _ in 0..5 {
        let started = std::time::Instant::now();
        let image = paint::draw(picture, buffer);
        times.push(started.elapsed().as_secs_f64() * 1e3);
        std::hint::black_box(image);
    }
    times.sort_by(f64::total_cmp);
    say!(
        "screen {width}x{height} at {scale}: number {}x{} logical, buffer {}x{}, draw median {:.2} ms (of 5)",
        logical.0,
        logical.1,
        buffer.0,
        buffer.1,
        times[2]
    );
    Ok(())
}
