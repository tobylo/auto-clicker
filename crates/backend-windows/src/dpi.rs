//! Per-monitor DPI awareness (M5 brief decision 10). A process that is not
//! DPI aware is told scaled sizes: at 150 % the virtual desktop of a
//! 1920-pixel screen reads 1280 wide, and an absolute move lands off by the
//! scale. Per-monitor aware, every size and position the daemon reads is in
//! physical pixels.

use std::io;

use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};

/// Makes this process per-monitor DPI aware (version 2). Called before the
/// process has any window: Windows fixes a process's awareness once it is
/// set or a window exists.
///
/// # Errors
/// Windows refused, as it does when the awareness was already set.
#[allow(unsafe_code)]
pub fn become_per_monitor_aware() -> io::Result<()> {
    // SAFETY: plain call with one of Windows' own constants.
    if unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
