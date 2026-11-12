//! The window's texts as a Swedish window shows them, on Slint's software
//! renderer: no display needed.

use std::rc::Rc;

use auto_clicker_core::config::Reason;
use auto_clicker_core::controller::{Blocked, NoRecording};
use auto_clicker_core::hotkeys::Refusal;
use auto_clicker_core::hotkeys::Unavailable;
use auto_clicker_core::locale::Language;
use auto_clicker_core::message::{
    ChangeFailure, ConfigProblem, Failure, Hindrance, InputFailure, Notice, Problem, Quoted,
    RecordingChange, SaveFailure, Saving, Start, Step, Stop, Unavailability, Unplayable,
};
use auto_clicker_core::window::RecordingOutcome;
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
    refusals_and_failures(&said);
    problems(&said);
    recordings(&said);
}

/// A refusal, a failure and an unavailable, as a note.
fn refusals_and_failures(said: &Said<'_>) {
    let note = |note: Note| note_text(said, &note).to_string();
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
    // The window's own "{}: {}" keeps its own form.
    assert_eq!(
        note(Note::Refused(Field::Speed, Reason::Speed)),
        "Hastighet: måste vara från 0,25 till 4"
    );
}

/// Problems, with quoted text as it is.
fn problems(said: &Said<'_>) {
    let note = |note: Note| note_text(said, &note).to_string();
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
    assert_eq!(
        said.problem(&Problem::Input(InputFailure::Portal(Quoted::new(
            "timed out"
        )))),
        "Kan inte börja klicka: portalen för fjärrinmatning misslyckades: timed out"
    );
    assert_eq!(
        said.problem(&Problem::Hotkeys(Unavailable::NoKeyboard)),
        "snabbtangenterna är inte tillgängliga: inget tangentbord hittades"
    );
    assert_eq!(
        said.no_recording(&NoRecording::DesktopHotkeys),
        "inspelning går inte med skrivbordets genvägar"
    );
    assert_eq!(
        note(Note::SavedUnsynced(Quoted::new("EIO"))),
        "Sparat, men det kanske inte klarar ett strömavbrott: EIO"
    );
}

/// What Play, Rename and Delete answer.
fn recordings(said: &Said<'_>) {
    let note = |note: Note| note_text(said, &note).to_string();
    let not_playing = RecordingOutcome::NotPlaying(Some(Notice::Unplayable {
        recording: "Farm loop".to_owned(),
        why: Unplayable::TooShort { ms: 20 },
    }));
    assert_eq!(
        note(Note::Recording(not_playing)),
        "Spelar inte upp Farm loop: den är kortare än 20 ms, för kort för att spelas mer än en \
         gång"
    );
    let recorded = RecordingOutcome::NotPlaying(Some(Notice::Recorded {
        stop: Some(Stop::Blocked(Blocked::Locked)),
        saving: Saving::Failed(SaveFailure::NoDataDirectory),
    }));
    assert_eq!(
        note(Note::Recording(recorded)),
        "Stoppat: skärmen är låst. Inspelningen kunde inte sparas: ingen datamapp: varken \
         XDG_DATA_HOME eller HOME är satt"
    );
    let gone = RecordingOutcome::NotChanged(RecordingChange::Delete, ChangeFailure::Missing);
    assert_eq!(
        note(Note::Recording(gone)),
        "Inte borttagen: filen finns inte längre"
    );
    let full = RecordingOutcome::NotChanged(
        RecordingChange::Rename,
        ChangeFailure::NotDone {
            step: Step::Written,
            why: Quoted::new("No space left on device (os error 28)"),
        },
    );
    assert_eq!(
        note(Note::Recording(full)),
        "Bytte inte namn: filen kunde inte skrivas: No space left on device (os error 28)"
    );
    let own_key = Notice::Refused {
        start: Start::Playback,
        why: Hindrance::OwnKey(Refusal::Modifier(
            auto_clicker_core::input::Key::new(42).unwrap(),
        )),
    };
    assert_eq!(
        said.notice(&own_key),
        "Spelar inte upp: leftshift är en modifierare, och om programmet håller den ändras \
         vilken snabbtangent skrivbordet ser"
    );
}
