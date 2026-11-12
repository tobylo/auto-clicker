//! The window's texts as a Swedish window shows them, on Slint's software
//! renderer: no display needed.

use std::rc::Rc;

use auto_clicker_core::config::Reason;
use auto_clicker_core::controller::Blocked;
use auto_clicker_core::locale::Language;
use auto_clicker_core::message::{ConfigProblem, Failure, Problem, Quoted, Unavailability};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};

use super::*;
use crate::ui::SettingsWindow;

struct Headless(Rc<MinimalSoftwareWindow>);

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// One test, since Slint's platform is set once for the process and its
/// windows live on the thread that made them.
#[test]
fn a_swedish_window_says_the_daemons_messages_in_swedish() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(window))).unwrap();
    // As the window's main does: the translations are there once a window is.
    let window = SettingsWindow::new().unwrap();
    slint::select_bundled_translation(Language::Swedish.code()).unwrap();
    let texts = window.global::<Texts>();
    let said = Said {
        texts: &texts,
        decimal: ',',
    };
    let note = |note: Note| note_text(&said, &note).to_string();
    let interval = Reason::Interval {
        floor: 10,
        max: 86_400_000,
        double: false,
    };
    assert_eq!(
        note(Note::Refused(Field::Interval, interval)),
        "Intervall: måste vara från 10 till 86400000 ms"
    );
    let rate = Reason::Rate {
        floor: 20,
        max: 86_400_000,
        double: true,
    };
    assert_eq!(
        note(Note::Refused(Field::Interval, rate)),
        "Intervall: måste vara från 0,000012 till 50 åtgärder i sekunden för ett \
         dubbelklick, som rymmer två klick i varje intervall"
    );
    // Text from another program, as it is.
    let disk = Failure::Write(Quoted::new("No space left on device (os error 28)"));
    assert_eq!(
        note(Note::Failed(disk)),
        "Inte sparat: konfigurationsfilen kunde inte skrivas: No space left on device (os \
         error 28)"
    );
    assert_eq!(
        note(Note::Unavailable(Unavailability::Blocked(Blocked::Locked))),
        "Inte tillgängligt: skärmen är låst"
    );
    let invalid = |line| {
        Problem::Config(ConfigProblem::Invalid {
            file: Quoted::new("config.toml"),
            line,
            why: Quoted::new("expected a boolean"),
        })
    };
    assert_eq!(
        said.problem(&invalid(Some(4))),
        "config.toml, rad 4: expected a boolean"
    );
    assert_eq!(
        said.problem(&invalid(None)),
        "config.toml gick inte att använda: expected a boolean"
    );
    // The window's own "{}: {}" keeps its own form.
    assert_eq!(
        note(Note::Refused(Field::Speed, Reason::Speed)),
        "Hastighet: måste vara från 0,25 till 4"
    );
}
