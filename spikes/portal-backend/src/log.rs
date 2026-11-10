//! Output lines stamped with CLOCK_MONOTONIC, the clock libei frames and
//! Wayland event times share, so the spike's lines line up with the probe
//! window's WAYLAND_DEBUG log.

use rustix::time::{ClockId, clock_gettime};

/// Microseconds on CLOCK_MONOTONIC.
pub fn now_us() -> u64 {
    let now = clock_gettime(ClockId::Monotonic);
    let secs = u64::try_from(now.tv_sec).unwrap_or(0);
    let nanos = u64::try_from(now.tv_nsec).unwrap_or(0);
    secs.saturating_mul(1_000_000).saturating_add(nanos / 1_000)
}

/// Prints one line to stdout, flushed, with a millisecond monotonic stamp.
pub fn line(text: &str) {
    let us = now_us();
    println!("[{}.{:03}] {text}", us / 1_000, us % 1_000);
}
