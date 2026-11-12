//! The settings window of auto-clicker. The daemon starts it from the tray
//! with a socket as its standard input; it shows what the daemon sends, and
//! sends the user's changes back for the daemon to check and save. It
//! exits when the user closes it or the daemon closes the socket.

mod app;
mod link;
mod model;
mod pick;
mod said;
mod shelf;
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
    // The overlays that pick the fixed point are made full-screen on their
    // screens as they are created, through the backend's hook.
    let backend = i_slint_backend_winit::Backend::builder()
        .with_window_attributes_hook(pick::hook)
        .build();
    if let Err(err) = backend.and_then(|backend| {
        slint::platform::set_platform(Box::new(backend)).map_err(|err| err.to_string().into())
    }) {
        return fail(&format!("could not start the window system: {err}"));
    }
    // The desktop file's id, so the desktop names the window and gives it
    // the program's icon.
    let _ = slint::set_xdg_app_id(APP_ID);
    let window = match SettingsWindow::new() {
        Ok(window) => window,
        Err(err) => return fail(&format!("could not open the window: {err}")),
    };
    // English is the text as written; another language that has no
    // translation bundled shows English too.
    let language = Language::of_environment();
    let _ = slint::select_bundled_translation(language.code());
    let decimal = match language {
        Language::English => '.',
        Language::Swedish => ',',
    };
    let app = App::start(&window, link, decimal);
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

#[cfg(test)]
mod tests {
    /// The string literal at the start of `text`, still escaped as Slint
    /// and the .po file both escape it, and what follows it.
    fn literal(text: &str) -> Option<(&str, &str)> {
        let body = text.strip_prefix('"')?;
        let mut escaped = false;
        let end = body.char_indices().find_map(|(at, c)| match (escaped, c) {
            (false, '\\') => {
                escaped = true;
                None
            }
            (false, '"') => Some(at),
            _ => {
                escaped = false;
                None
            }
        })?;
        Some((&body[..end], &body[end + 1..]))
    }

    /// Every `@tr` text of `ui`, with its context if it has one.
    fn texts(ui: &str) -> Vec<(Option<&str>, &str)> {
        ui.split("@tr(")
            .skip(1)
            .filter_map(|rest| {
                let (first, after) = literal(rest)?;
                match after.strip_prefix(" => ").and_then(literal) {
                    Some((text, _)) => Some((Some(first), text)),
                    None => Some((None, first)),
                }
            })
            .collect()
    }

    /// Every text the window shows has a Swedish translation: a missing one
    /// would show English in a Swedish window.
    #[test]
    fn every_text_is_translated_to_swedish() {
        let ui = [
            include_str!("../ui/window.slint"),
            include_str!("../ui/pick.slint"),
        ]
        .concat();
        let po = include_str!("../translations/sv/LC_MESSAGES/auto-clicker-settings.po");
        let texts = texts(&ui);
        assert!(texts.len() > 100, "{}", texts.len());
        for (context, text) in texts {
            let context = context
                .map(|c| format!("msgctxt \"{c}\"\n"))
                .unwrap_or_default();
            let entry = format!("\n{context}msgid \"{text}\"\nmsgstr ");
            let translated = po.split(&entry).nth(1).and_then(literal);
            assert!(
                translated.is_some_and(|(swedish, _)| !swedish.is_empty()),
                "{text}"
            );
        }
    }

    #[test]
    fn texts_are_read_with_their_context_and_escapes() {
        let ui = r#"@tr("a \"b\" c") @tr("ctx" => "{}: {}", x) @tr("d")"#;
        assert_eq!(
            texts(ui),
            [(None, r#"a \"b\" c"#), (Some("ctx"), "{}: {}"), (None, "d")]
        );
    }
}
