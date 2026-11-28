//! The settings window on Linux. The daemon starts it from the tray with a
//! socket as its standard input.

mod link;
pub mod place;

use std::process::ExitCode;

use auto_clicker_core::locale::Language;

use crate::{fail, run};

/// The program's id, as its desktop file is named.
const APP_ID: &str = "io.github.tobylo.AutoClicker";

/// Brings `window` forward when the daemon asks: on Wayland the desktop
/// decides, so showing it is all the window does.
pub fn bring_forward(_window: &slint::Window) {}

/// Shows the window until the user closes it or the daemon goes.
pub fn main() -> ExitCode {
    let (link, reader) = match link::from_stdin() {
        Ok(link) => link,
        Err(err) => return fail(&err.to_string()),
    };
    // The overlays that pick the fixed point are made full-screen on their
    // screens as they are created, through the backend's hook.
    let backend = i_slint_backend_winit::Backend::builder()
        .with_window_attributes_hook(crate::pick::hook)
        .build();
    if let Err(err) = backend.and_then(|backend| {
        slint::platform::set_platform(Box::new(backend)).map_err(|err| err.to_string().into())
    }) {
        return fail(&format!("could not start the window system: {err}"));
    }
    // The desktop file's id, so the desktop names the window and gives it
    // the program's icon.
    let _ = slint::set_xdg_app_id(APP_ID);
    run(link, reader, Language::of_environment())
}
