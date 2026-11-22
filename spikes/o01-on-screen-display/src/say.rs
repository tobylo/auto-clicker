//! What the probe heard, one line each, stamped on `CLOCK_MONOTONIC` so a
//! driver reading `time.monotonic()` can line it up with what it injected.

use std::io::Write;

use rustix::time::{ClockId, clock_gettime};

/// Seconds on `CLOCK_MONOTONIC`.
pub fn now() -> f64 {
    let time = clock_gettime(ClockId::Monotonic);
    // Seconds since boot fit an f64's 52 bits for millennia.
    time.tv_sec as f64 + f64::from(u32::try_from(time.tv_nsec).unwrap_or(0)) / 1e9
}

/// Prints `line` with the time, and flushes so a pipe sees it at once.
pub fn line(text: &str) {
    let mut out = std::io::stdout().lock();
    // A closed pipe means the driver is gone; nothing to report it to.
    let _ = writeln!(out, "{:.4} {text}", now());
    let _ = out.flush();
}

/// Prints a line built from format arguments.
#[macro_export]
macro_rules! say {
    ($($arg:tt)*) => { $crate::say::line(&format!($($arg)*)) };
}

/// The process's resident memory and context switches, from
/// `/proc/self/status`: "rss_kib=… voluntary=… involuntary=…".
pub fn stats() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map(|rest| rest.trim().trim_end_matches(" kB").to_owned())
            .unwrap_or_else(|| "?".to_owned())
    };
    format!(
        "rss_kib={} voluntary={} involuntary={}",
        field("VmRSS:"),
        field("voluntary_ctxt_switches:"),
        field("nonvoluntary_ctxt_switches:")
    )
}
