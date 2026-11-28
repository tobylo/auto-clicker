//! The settings window on Windows. The daemon starts it from the tray with
//! two pipes named on its command line, and no standard handles: what a
//! library prints goes nowhere, and never reaches the daemon (M5 brief
//! decision 14). It logs to a file of its own beside the daemon's, since it
//! has no console.

mod keys;
pub mod place;

use std::process::ExitCode;

use auto_clicker_backend_windows::settings_window::{self, take_inherited};
use auto_clicker_backend_windows::{become_per_monitor_aware, display_languages};
use auto_clicker_core::dirs::log_path;
use auto_clicker_core::locale::Language;
use auto_clicker_core::log;

use crate::link::Link;
use crate::{fail, run};

/// The window's log, beside the daemon's.
const LOG: &str = "auto-clicker-settings.log";

/// Brings `window` to the front, restored if it was minimised, as far as
/// the daemon let it (it does when Settings is chosen in its tray).
pub fn bring_forward(window: &slint::Window) {
    use i_slint_backend_winit::WinitWindowAccessor;
    use i_slint_backend_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let hwnd = window.with_winit_window(|winit| match winit.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    });
    if let Some(Some(hwnd)) = hwnd {
        settings_window::bring_forward(hwnd);
    }
}

/// Shows the window until the user closes it or the daemon goes.
pub fn main() -> ExitCode {
    if let Some(path) = log_path() {
        // Lines go nowhere if it cannot be opened: there is no console.
        let _ = log::to_file(&path.with_file_name(LOG));
    }
    // Before any window exists: the overlays that pick the fixed point
    // cover their monitors in physical pixels (decision 10). winit asks
    // for the same, so a refusal here changes nothing.
    if let Err(err) = become_per_monitor_aware() {
        log::line(format_args!(
            "auto-clicker-settings: not per-monitor DPI aware ({err})"
        ));
    }
    let (requests, updates) = match take_inherited() {
        Ok(ends) => ends,
        Err(err) => {
            return fail(&format!(
                "the daemon's pipes cannot be used ({err}): it opens from the tray"
            ));
        }
    };
    let backend = i_slint_backend_winit::Backend::builder()
        .with_window_attributes_hook(crate::pick::hook)
        .with_custom_application_handler(Box::new(keys::Keys::default()))
        .build();
    if let Err(err) = backend.and_then(|backend| {
        slint::platform::set_platform(Box::new(backend)).map_err(|err| err.to_string().into())
    }) {
        return fail(&format!("could not start the window system: {err}"));
    }
    // The user's display languages, as the daemon's tray reads them.
    let language = display_languages().map_or(Language::English, |names| {
        Language::from_tags(names.iter().map(String::as_str))
    });
    run(Link::new(requests), updates, language)
}
