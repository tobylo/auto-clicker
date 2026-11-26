//! The opt-out from execution-speed power throttling, held while any run
//! is active. Windows may slow the timers of a process with no visible
//! window, which the daemon is; idle, the program leaves the choice to
//! Windows again (M5 brief decision 5).

use std::io;
use std::sync::{Mutex, PoisonError};

use auto_clicker_core::log;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    ProcessPowerThrottling, SetProcessInformation,
};

/// How many runs hold the opt-out. It is the process's, so the first run
/// sets it and the last one undoes it.
static HOLDERS: Mutex<u32> = Mutex::new(0);

/// One run's hold on the opt-out.
pub(super) struct Throttling {
    _private: (),
}

impl Throttling {
    /// Holds the opt-out. A failure to set it is reported and the run goes
    /// on: it only affects timing.
    pub(super) fn opt_out() -> Throttling {
        let mut holders = HOLDERS.lock().unwrap_or_else(PoisonError::into_inner);
        if *holders == 0
            && let Err(err) = set(true)
        {
            log::line(format_args!(
                "auto-clicker: could not opt out of power throttling for the run: {err}"
            ));
        }
        *holders = holders.saturating_add(1);
        Throttling { _private: () }
    }
}

impl Drop for Throttling {
    fn drop(&mut self) {
        let mut holders = HOLDERS.lock().unwrap_or_else(PoisonError::into_inner);
        *holders = holders.saturating_sub(1);
        if *holders == 0
            && let Err(err) = set(false)
        {
            log::line(format_args!(
                "auto-clicker: could not hand power throttling back to Windows: {err}"
            ));
        }
    }
}

/// Opts out of execution-speed throttling, or hands it back to Windows.
#[allow(unsafe_code)]
fn set(opt_out: bool) -> io::Result<()> {
    let control = if opt_out {
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED
    } else {
        0
    };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        // Controlled with the state bit clear: never throttled. Not
        // controlled: Windows decides.
        ControlMask: control,
        StateMask: 0,
    };
    // SAFETY: the current process's pseudo handle, and a valid state struct
    // with its size, alive for the call.
    let set = unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&raw const state).cast(),
            state_size(),
        )
    };
    if set == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Whether Windows manages execution-speed throttling for this process, as
/// it does when no run is active.
///
/// # Errors
/// When Windows cannot report it.
#[cfg(test)]
pub(crate) fn execution_speed_managed() -> io::Result<bool> {
    Ok(read_state()?.ControlMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED == 0)
}

/// The process's power throttling state, as Windows reports it.
#[cfg(test)]
#[allow(unsafe_code)]
fn read_state() -> io::Result<PROCESS_POWER_THROTTLING_STATE> {
    use windows_sys::Win32::System::Threading::GetProcessInformation;

    let mut state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ..PROCESS_POWER_THROTTLING_STATE::default()
    };
    // SAFETY: the current process's pseudo handle, and a state struct with
    // its size that the call fills in, alive for the call.
    let read = unsafe {
        GetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&raw mut state).cast(),
            state_size(),
        )
    };
    if read == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(state)
}

/// Whether this process has opted out of execution-speed throttling: the
/// control bit set and the state bit clear (both set would ask for it).
///
/// # Errors
/// When Windows cannot report it.
#[cfg(test)]
pub(crate) fn execution_speed_opted_out() -> io::Result<bool> {
    let state = read_state()?;
    let bit = PROCESS_POWER_THROTTLING_EXECUTION_SPEED;
    Ok(state.ControlMask & bit != 0 && state.StateMask & bit == 0)
}

fn state_size() -> u32 {
    // Twelve bytes.
    u32::try_from(size_of::<PROCESS_POWER_THROTTLING_STATE>()).unwrap_or(u32::MAX)
}
