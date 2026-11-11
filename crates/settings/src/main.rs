//! The settings window of auto-clicker. The daemon starts it from the tray
//! with a socket as its standard input; it shows what the daemon sends, and
//! sends the user's changes back for the daemon to check and save. It
//! exits when the user closes it or the daemon closes the socket.

mod app;
mod link;
mod model;
mod state;

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;

use auto_clicker_core::locale::Language;
use slint::ComponentHandle;

use app::App;

/// The code Slint generates from `ui/window.slint`. It is not ours to
/// change, and unwraps what its own construction guarantees.
#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used
)]
mod ui {
    slint::include_modules!();
}

use ui::SettingsWindow;

/// The program's id, as its desktop file is named.
const APP_ID: &str = "io.github.tobylo.AutoClicker";

thread_local! {
    /// The app, on the window's thread, where updates from the daemon are
    /// handed to it.
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

fn main() -> ExitCode {
    let (link, reader) = match link::Link::from_stdin() {
        Ok(link) => link,
        Err(err) => return fail(&err.to_string()),
    };
    // The desktop file's id, so the desktop names the window and gives it
    // the program's icon.
    let _ = slint::set_xdg_app_id(APP_ID);
    let window = match SettingsWindow::new() {
        Ok(window) => window,
        Err(err) => return fail(&format!("could not open the window: {err}")),
    };
    // English is the text as written; another language that has no
    // translation bundled shows English too.
    let _ = slint::select_bundled_translation(Language::of_environment().code());
    let app = App::start(&window, link);
    APP.with(|slot| *slot.borrow_mut() = Some(app));
    std::thread::spawn(move || {
        link::read_updates(reader, |update| {
            let _ = slint::invoke_from_event_loop(move || {
                APP.with(|slot| {
                    if let Some(app) = slot.borrow().as_ref() {
                        app.borrow_mut().update(update);
                    }
                });
            });
        });
        // The daemon closed its end, or sent what is no update.
        let _ = slint::invoke_from_event_loop(|| {
            let _ = slint::quit_event_loop();
        });
    });
    match window.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&format!("the window failed: {err}")),
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("auto-clicker-settings: {message}");
    ExitCode::FAILURE
}
