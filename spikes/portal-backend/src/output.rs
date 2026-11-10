//! Output scenarios on a RemoteDesktop session through libei.

use std::path::Path;
use std::time::Duration;

use reis::event::DeviceCapability;
use tokio::time::{Instant, sleep, sleep_until};

use crate::eis::Output;
use crate::log::line;
use crate::number;
use crate::remote::Remote;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const KEY_A: u32 = 30;
const KEY_LEFTCTRL: u32 = 29;
const KEY_F6: u32 = 64;
const PAUSE: Duration = Duration::from_millis(300);

pub async fn run(token_file: &Path, args: &[String]) -> Result<(), String> {
    let (remote, fd) = Remote::open(token_file).await?;
    if matches!(args, [name] if name == "grant") {
        return remote.close().await;
    }
    let mut out = Output::connect(fd).await?;
    // KWin announces all three devices at once; wait for every one a
    // scenario may use.
    out.wait_for(&[
        DeviceCapability::Button,
        DeviceCapability::Keyboard,
        DeviceCapability::Pointer,
        DeviceCapability::PointerAbsolute,
    ])
    .await?;
    out.describe();
    scenario(&mut out, &remote, args).await
}

async fn scenario(out: &mut Output, remote: &Remote, args: &[String]) -> Result<(), String> {
    match args {
        [name] if name == "demo" => demo(out).await,
        [name, n, ms] if name == "rate" => rate(out, number(n)?, number(ms)?).await,
        [name, what, how] if name == "hold" => hold(out, remote, what, how).await,
        [name] if name == "combo" => combo(out).await,
        [name] if name == "locked" => locked(out).await,
        [name] if name == "park" => park(out).await,
        _ => Err(format!("unknown output scenario {args:?}")),
    }
}

async fn step(label: &str, result: Result<(), String>) -> Result<(), String> {
    line(&format!("sent {label}: {result:?}"));
    sleep(PAUSE).await;
    result
}

async fn click(out: &mut Output, code: u32, label: &str) -> Result<(), String> {
    step(&format!("{label} down"), out.button(code, true)).await?;
    step(&format!("{label} up"), out.button(code, false)).await
}

async fn demo(out: &mut Output) -> Result<(), String> {
    absolute_demo(out).await?;
    click(out, BTN_LEFT, "left").await?;
    click(out, BTN_RIGHT, "right").await?;
    click(out, BTN_MIDDLE, "middle").await?;
    step("key A down", out.key(KEY_A, true)).await?;
    step("key A up", out.key(KEY_A, false)).await?;
    step("relative +100,+50", out.move_relative(100.0, 50.0)).await?;
    step("relative -100,-50", out.move_relative(-100.0, -50.0)).await?;
    step("scroll 1 notch down", out.scroll_notches(1)).await?;
    step("scroll 1 notch up", out.scroll_notches(-1)).await
}

/// Moves to the centre and then the top-left quarter point of the first
/// region, in the region's own (logical pixel) coordinates.
async fn absolute_demo(out: &mut Output) -> Result<(), String> {
    let regions = out.absolute_regions();
    line(&format!("absolute regions: {regions:?}"));
    let Some(&(x, y, width, height)) = regions.first() else {
        line("no absolute pointer region: skipping absolute motion");
        return Ok(());
    };
    let (x, y, w, h) = (x as f32, y as f32, width as f32, height as f32);
    step(
        "absolute centre",
        out.move_absolute(x + w / 2.0, y + h / 2.0),
    )
    .await?;
    step(
        "absolute quarter",
        out.move_absolute(x + w / 4.0, y + h / 4.0),
    )
    .await?;
    step(
        "absolute centre",
        out.move_absolute(x + w / 2.0, y + h / 2.0),
    )
    .await
}

/// Moves the pointer to the centre of the first region, so a window opened
/// next opens on that output.
async fn park(out: &mut Output) -> Result<(), String> {
    let regions = out.absolute_regions();
    let &(x, y, width, height) = regions.first().ok_or("no absolute region")?;
    let centre = (
        x as f32 + width as f32 / 2.0,
        y as f32 + height as f32 / 2.0,
    );
    step("park at centre", out.move_absolute(centre.0, centre.1)).await
}

/// `count` left clicks on absolute deadlines, each held for half the interval.
async fn rate(out: &mut Output, count: u64, interval_ms: u64) -> Result<(), String> {
    let interval = Duration::from_millis(interval_ms);
    let hold = interval / 2;
    let start = Instant::now() + Duration::from_millis(500);
    let mut failures = 0u64;
    for i in 0..count {
        let due = start + interval.saturating_mul(u32::try_from(i).unwrap_or(u32::MAX));
        sleep_until(due).await;
        failures += u64::from(out.button(BTN_LEFT, true).is_err());
        sleep_until(due + hold).await;
        failures += u64::from(out.button(BTN_LEFT, false).is_err());
    }
    let elapsed = start.elapsed();
    line(&format!(
        "rate: sent {count} clicks at {interval_ms} ms in {} ms, {failures} failed writes",
        elapsed.as_millis()
    ));
    // Let the last frames reach the window before the session closes.
    sleep(Duration::from_secs(1)).await;
    Ok(())
}

/// Presses and holds, then ends the session one way, and reports what a
/// write afterwards returns.
async fn hold(out: &mut Output, remote: &Remote, what: &str, how: &str) -> Result<(), String> {
    let (code, is_key) = match what {
        "button" => (BTN_LEFT, false),
        "key" => (KEY_A, true),
        _ => return Err(format!("hold what? {what}")),
    };
    let press = |out: &mut Output, down| {
        if is_key {
            out.key(code, down)
        } else {
            out.button(code, down)
        }
    };
    line(&format!("holding {what}: {:?}", press(out, true)));
    sleep(Duration::from_secs(2)).await;
    end(out, remote, how).await?;
    // Give the compositor time to act (KWin's stop-emulating timer is 1 s).
    sleep(Duration::from_secs(3)).await;
    line(&format!(
        "after {how}: disconnected = {:?}",
        out.disconnected()
    ));
    line(&format!("write after {how}: {:?}", press(out, false)));
    Ok(())
}

async fn end(out: &mut Output, remote: &Remote, how: &str) -> Result<(), String> {
    line(&format!("ending by {how}"));
    match how {
        // The harness sends SIGKILL now; this sleep never ends.
        "kill" => {
            sleep(Duration::from_secs(30)).await;
            Ok(())
        }
        "close" => remote.close().await,
        "eis" => out.disconnect(),
        "stop" => out.stop_emulating(),
        // Returns with the button held; the process exits normally.
        "exit" => std::process::exit(0),
        _ => Err(format!("end how? {how}")),
    }
}

/// Ctrl+F6 through libei, to see whether GlobalShortcuts fires on it.
async fn combo(out: &mut Output) -> Result<(), String> {
    step("ctrl down", out.key(KEY_LEFTCTRL, true)).await?;
    step("F6 down", out.key(KEY_F6, true)).await?;
    step("F6 up", out.key(KEY_F6, false)).await?;
    step("ctrl up", out.key(KEY_LEFTCTRL, false)).await
}

/// For a game that has locked the pointer: turn by relative motion, then
/// right-click ten times slowly and left-click ten times at the 10 ms floor.
async fn locked(out: &mut Output) -> Result<(), String> {
    for _ in 0..10 {
        step("relative +40,0", out.move_relative(40.0, 0.0)).await?;
    }
    for _ in 0..10 {
        click(out, BTN_RIGHT, "right").await?;
    }
    rate(out, 100, 10).await
}
