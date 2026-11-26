//! Keeping the display on during a run (M5 brief decision 5). The display
//! turning off stops a run, and a hold sends no input to keep it on, so the
//! run timer's thread asks Windows to keep it on for as long as it lives.
//! The request belongs to the thread: Windows drops it when the thread ends,
//! however it ends, and the thread clears it itself on its way out.

use auto_clicker_core::log;
use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, SetThreadExecutionState,
};

/// This thread's request to keep the display on, until dropped.
pub(super) struct DisplayOn {
    _private: (),
}

impl DisplayOn {
    /// Asks Windows to keep the display on while this thread lives. A
    /// refusal is logged and the run goes on.
    #[allow(unsafe_code)]
    pub(super) fn request() -> DisplayOn {
        // SAFETY: plain flags, for the calling thread.
        if unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED) } == 0 {
            log::line(format_args!(
                "auto-clicker: Windows would not keep the display on during the run"
            ));
        }
        DisplayOn { _private: () }
    }
}

impl Drop for DisplayOn {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: plain flags, for the calling thread, the one that asked.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

/// Whether some thread asks Windows to keep the display on.
///
/// # Errors
/// When Windows cannot report it.
#[cfg(test)]
#[allow(unsafe_code)]
pub(crate) fn display_kept_on() -> std::io::Result<bool> {
    use windows_sys::Win32::System::Power::{CallNtPowerInformation, SystemExecutionState};

    let mut state: u32 = 0;
    let size = u32::try_from(size_of::<u32>()).unwrap_or(u32::MAX);
    // SAFETY: no input; the output is one u32 with its size, alive for the
    // call.
    let status = unsafe {
        CallNtPowerInformation(
            SystemExecutionState,
            std::ptr::null(),
            0,
            (&raw mut state).cast(),
            size,
        )
    };
    if status != 0 {
        return Err(std::io::Error::other(format!(
            "CallNtPowerInformation returned {status:#x}"
        )));
    }
    Ok(state & ES_DISPLAY_REQUIRED != 0)
}
