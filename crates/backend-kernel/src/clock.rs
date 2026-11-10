//! The clocks recordings need: the monotonic clock input events are stamped
//! with, and the local time that names a recording.

use std::time::Duration;

use auto_clicker_core::recording::LocalTime;

/// The monotonic clock now, as input event times count it.
#[allow(unsafe_code)]
pub fn monotonic_now() -> Duration {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a live, writable `timespec` for the call to fill.
    // `CLOCK_MONOTONIC` always exists, so the call cannot fail; were it to,
    // `now` stays zero, which suppresses nothing.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
    let seconds = u64::try_from(now.tv_sec).unwrap_or(0);
    let nanos = u32::try_from(now.tv_nsec).unwrap_or(0);
    Duration::new(seconds, nanos)
}

#[allow(unsafe_code)]
unsafe extern "C" {
    /// POSIX `tzset` (time.h), which the `libc` crate does not declare: reads
    /// the time zone again from `TZ` or `/etc/localtime`.
    fn tzset();
}

/// The local date and time now, in the time zone the system is set to
/// (`TZ`, else `/etc/localtime`, read again on each call); `None` when the
/// clock or the zone cannot be read.
#[allow(unsafe_code)]
pub fn local_time() -> Option<LocalTime> {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a live, writable `timespec` for the call to fill.
    if unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut now) } != 0 {
        return None;
    }
    // SAFETY: an all-zero `tm` is a valid value of the plain C struct; its
    // one pointer, `tm_zone`, is null until `localtime_r` sets it.
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `tzset` reads the environment and the zone file; this program
    // never changes its environment, so no other thread writes it meanwhile.
    // `localtime_r` reads `now.tv_sec` and fills `parts`, both live for the
    // call, and keeps neither; it returns null on failure.
    let filled = unsafe {
        tzset();
        libc::localtime_r(&now.tv_sec, &mut parts)
    };
    if filled.is_null() {
        return None;
    }
    let field = |value: libc::c_int| u32::try_from(value).ok();
    Some(LocalTime {
        year: i64::from(parts.tm_year).checked_add(1900)?,
        month: field(parts.tm_mon)?.checked_add(1)?,
        day: field(parts.tm_mday)?,
        hour: field(parts.tm_hour)?,
        minute: field(parts.tm_min)?,
        second: field(parts.tm_sec)?,
        millis: u32::try_from(now.tv_nsec / 1_000_000).ok()?,
        offset_seconds: parts.tm_gmtoff,
    })
}

#[cfg(test)]
mod tests {
    use auto_clicker_core::recording::Created;

    use super::*;

    #[test]
    fn the_monotonic_clock_moves_on() {
        let first = monotonic_now();
        assert!(first > Duration::ZERO);
        assert!(monotonic_now() >= first);
    }

    #[test]
    fn the_local_time_is_a_real_time() {
        let now = local_time().unwrap();
        let created = Created::new(now).unwrap();
        assert!(now.year >= 2026, "{now:?}");
        assert!(created.to_string().len() == 29, "{created}");
    }
}
