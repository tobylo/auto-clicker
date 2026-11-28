// No console on Windows: the window is started by the daemon, and logs to a
// file there.
#![cfg_attr(not(test), windows_subsystem = "windows")]

//! The settings window of auto-clicker, started by the daemon from its tray.
//! It shows what the daemon sends, and sends the user's changes back for
//! the daemon to check and save; it exits when the user closes it or the
//! daemon goes. The window is the same on both systems; each system's
//! module hands it its link to the daemon and sets up Slint's platform.

mod app;
mod link;
mod model;
mod pick;
mod said;
mod shelf;
mod state;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as system;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as system;

use std::cell::RefCell;
use std::fs::File;
use std::process::ExitCode;
use std::rc::Rc;

use auto_clicker_core::locale::Language;
use auto_clicker_core::log;
use slint::ComponentHandle;

use app::App;
use link::Link;

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

thread_local! {
    /// The app, on the window's thread, where updates from the daemon are
    /// handed to it.
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

fn main() -> ExitCode {
    system::main()
}

/// Shows the window in `language` until the user closes it or the daemon
/// goes: requests go out on `link`, updates come in on `updates`. Slint's
/// platform is set.
fn run(link: Link, updates: File, language: Language) -> ExitCode {
    let window = match SettingsWindow::new() {
        Ok(window) => window,
        Err(err) => return fail(&format!("could not open the window: {err}")),
    };
    // English is the text as written; another language that has no
    // translation bundled shows English too.
    let _ = slint::select_bundled_translation(language.code());
    let decimal = match language {
        Language::English => '.',
        Language::Swedish => ',',
    };
    let app = App::start(&window, link, decimal);
    APP.with(|slot| *slot.borrow_mut() = Some(app));
    std::thread::spawn(move || {
        link::read_updates(updates, |update| {
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

/// Logs `message` and fails.
fn fail(message: &str) -> ExitCode {
    log::line(format_args!("auto-clicker-settings: {message}"));
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

    /// A text the window shows: its context, if it has one, the text,
    /// and its plural, if it has one.
    #[derive(Debug, PartialEq)]
    struct Text<'a> {
        context: Option<&'a str>,
        text: &'a str,
        plural: Option<&'a str>,
    }

    /// Every `@tr` text of `ui`.
    fn texts(ui: &str) -> Vec<Text<'_>> {
        ui.split("@tr(")
            .skip(1)
            .filter_map(|rest| {
                let (first, after) = literal(rest)?;
                let (context, text, after) = match after.strip_prefix(" => ").and_then(literal) {
                    Some((text, after)) => (Some(first), text, after),
                    None => (None, first, after),
                };
                let plural = after.strip_prefix(" | ").and_then(literal).map(|(p, _)| p);
                Some(Text {
                    context,
                    text,
                    plural,
                })
            })
            .collect()
    }

    /// The translation that follows `entry` in `po`, when it is there and
    /// not empty.
    fn translated<'a>(po: &'a str, entry: &str) -> Option<&'a str> {
        let (swedish, _) = po.split(entry).nth(1).and_then(literal)?;
        (!swedish.is_empty()).then_some(swedish)
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
        for Text {
            context,
            text,
            plural,
        } in texts
        {
            let context = context
                .map(|c| format!("msgctxt \"{c}\"\n"))
                .unwrap_or_default();
            let entry = format!("\n{context}msgid \"{text}\"\n");
            match plural {
                None => assert!(
                    translated(po, &format!("{entry}msgstr ")).is_some(),
                    "{text}"
                ),
                Some(plural) => {
                    let entry = format!("{entry}msgid_plural \"{plural}\"\n");
                    let one = translated(po, &format!("{entry}msgstr[0] "));
                    let many = translated(
                        po,
                        &format!(
                            "{entry}msgstr[0] \"{}\"\nmsgstr[1] ",
                            one.unwrap_or_default()
                        ),
                    );
                    assert!(one.is_some() && many.is_some(), "{text}");
                }
            }
        }
    }

    #[test]
    fn texts_are_read_with_their_context_and_escapes() {
        let ui =
            r#"@tr("a \"b\" c") @tr("ctx" => "{}: {}", x) @tr("d") @tr("{n} e" | "{n} es" % n)"#;
        let text = |context, text, plural| Text {
            context,
            text,
            plural,
        };
        assert_eq!(
            texts(ui),
            [
                text(None, r#"a \"b\" c"#, None),
                text(Some("ctx"), "{}: {}", None),
                text(None, "d", None),
                text(None, "{n} e", Some("{n} es")),
            ]
        );
    }
}
