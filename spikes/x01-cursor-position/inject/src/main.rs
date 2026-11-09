//! X01 spike injector: moves C01's virtual mouse on command and timestamps
//! each move on `CLOCK_MONOTONIC`, the clock Python's `time.monotonic_ns`
//! reads.
//!
//! Commands on stdin, one per line; every reply ends with a line `done`:
//!
//! - `abs X Y`: one absolute move (0..=65535 per axis); replies `t 0 NS`.
//! - `rel DX DY`: one relative move; replies `t 0 NS`.
//! - `stream-abs HZ N X0 Y STEP`: N absolute moves to `X0 + i*STEP, Y`, paced
//!   at HZ on absolute deadlines; replies `t I NS` for each, after the run.
//! - `stream-rel HZ N D`: N relative moves of D px along x, turning round
//!   every 100 moves; replies as above.
//!
//! End of input or `quit` drops the devices, which the kernel then destroys.

use std::error::Error;
use std::io::{self, BufRead, Write};
use std::thread;
use std::time::{Duration, Instant};

use auto_clicker_backend_kernel::KernelBackend;
use auto_clicker_core::input::{AbsolutePosition, Input};
use nix::time::{ClockId, clock_gettime};

type Mouse = Input<KernelBackend>;
type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Moves per direction in a relative stream, so it never pins at an edge.
const LEG: u64 = 100;

fn main() -> Result<()> {
    let mut mouse = KernelBackend::open()?;
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.first() == Some(&"quit") {
            break;
        }
        let stamps = run(&mut mouse, &words)?;
        reply(&stamps)?;
    }
    Ok(())
}

fn run(mouse: &mut Mouse, words: &[&str]) -> Result<Vec<i64>> {
    match words {
        ["abs", x, y] => {
            let target = position(x.parse()?, y.parse()?);
            stamped(mouse.move_absolute(target))
        }
        ["rel", dx, dy] => {
            let (dx, dy) = (dx.parse()?, dy.parse()?);
            stamped(mouse.move_relative(dx, dy))
        }
        ["stream-abs", hz, n, x0, y, step] => {
            let (x0, y, step): (u32, u16, u32) = (x0.parse()?, y.parse()?, step.parse()?);
            paced(hz.parse()?, n.parse()?, |i| {
                let x = u64::from(x0) + i * u64::from(step);
                Ok(mouse.move_absolute(position(u16::try_from(x)?, y))?)
            })
        }
        ["stream-rel", hz, n, d] => {
            let d: i32 = d.parse()?;
            paced(hz.parse()?, n.parse()?, |i| {
                let dx = if (i / LEG).is_multiple_of(2) { d } else { -d };
                Ok(mouse.move_relative(dx, 0)?)
            })
        }
        _ => Err(format!("unknown command: {words:?}").into()),
    }
}

fn position(x: u16, y: u16) -> AbsolutePosition {
    AbsolutePosition { x, y }
}

fn stamped<E: Into<Box<dyn Error>>>(moved: std::result::Result<(), E>) -> Result<Vec<i64>> {
    moved.map_err(Into::into)?;
    Ok(vec![monotonic_ns()?])
}

/// Runs `step` n times at `hz`, each on its own absolute deadline, and
/// returns the time just after each step.
fn paced(hz: u64, n: u64, mut step: impl FnMut(u64) -> Result<()>) -> Result<Vec<i64>> {
    let period = Duration::from_secs(1)
        .checked_div(u32::try_from(hz)?)
        .ok_or("rate must be above zero")?;
    let start = Instant::now();
    let mut stamps = Vec::with_capacity(usize::try_from(n)?);
    for i in 0..n {
        let deadline = start + period.checked_mul(u32::try_from(i)?).ok_or("too long")?;
        thread::sleep(deadline.saturating_duration_since(Instant::now()));
        step(i)?;
        stamps.push(monotonic_ns()?);
    }
    Ok(stamps)
}

fn monotonic_ns() -> Result<i64> {
    let t = clock_gettime(ClockId::CLOCK_MONOTONIC)?;
    Ok(t.tv_sec() * 1_000_000_000 + t.tv_nsec())
}

fn reply(stamps: &[i64]) -> Result<()> {
    let mut out = io::stdout().lock();
    for (i, ns) in stamps.iter().enumerate() {
        writeln!(out, "t {i} {ns}")?;
    }
    writeln!(out, "done")?;
    out.flush()?;
    Ok(())
}
