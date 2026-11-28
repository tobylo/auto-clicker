//! The window's texts as a Swedish window shows them, on Slint's software
//! renderer: no display needed.

use std::rc::Rc;

use auto_clicker_core::config::Reason;
use auto_clicker_core::controller::{Blocked, NoRecording};
use auto_clicker_core::hotkeys::Refusal;
use auto_clicker_core::hotkeys::{Binding, NotRegistered, Unavailable, Unregistered};
use auto_clicker_core::locale::Language;
use auto_clicker_core::message::{
    ChangeFailure, ConfigProblem, Failure, Hindrance, InputFailure, NoOnScreen, NoPointer,
    NoPositions, NoScreens, Notice, PlayStart, PointScreens, Problem, Quoted, SaveFailure, Saving,
    Start, Step, Stop, Titled, Unavailability, Unplayable,
};
use auto_clicker_core::playback::Fallback;
use auto_clicker_core::recording::FileName;
use auto_clicker_core::window::{NotDeleted, RecordingOutcome, Removal};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};

use super::*;
use crate::ui::SettingsWindow;

mod list;
mod on_screen;

/// A platform giving every window a software window of its own.
struct Headless;

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let window: Rc<dyn WindowAdapter> =
            MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        Ok(window)
    }
}

/// One test, since Slint's platform is set once for the process and its
/// windows live on the thread that made them.
#[test]
fn a_swedish_window_says_the_daemons_messages_in_swedish() {
    slint::platform::set_platform(Box::new(Headless)).unwrap();
    // As the window's main does: the translations are there once a window is.
    let window = SettingsWindow::new().unwrap();
    slint::select_bundled_translation(Language::Swedish.code()).unwrap();
    let texts = window.global::<Texts>();
    let said = Said {
        texts: &texts,
        decimal: ',',
        windows: false,
    };
    windows_words(&Said {
        windows: true,
        ..said
    });
    refusals_and_failures(&said);
    problems(&said);
    no_positions(&said);
    no_on_screen(&said);
    recordings(&said);
    point_screens(&said);
    titles(&said);
    deletions(&said);
    crate::pick::tests::every_end_is_told_once_and_closes_every_overlay();
    list::the_list_tells_clicks_with_their_keys_and_the_keys_that_select();
    list::the_list_keeps_the_row_the_arrows_reached_in_view();
    list::the_actions_follow_the_selection();
    list::a_delete_asks_once_and_sends_the_selection_whole();
    on_screen::the_group_shows_the_switches_saves_them_and_says_why_it_cannot_show();
}

/// What a window on Windows says its own way: its folders, and the
/// refusals only Windows gives.
fn windows_words(said: &Said<'_>) {
    assert_eq!(
        said.problem(&Problem::Config(ConfigProblem::NoDirectory)),
        "ingen konfigurationsmapp: APPDATA är inte satt till en absolut mapp"
    );
    assert_eq!(
        said.failure(&Failure::NoConfigDirectory),
        "det finns ingen konfigurationsmapp: APPDATA är inte satt till en absolut mapp"
    );
    let recorded = Notice::Recorded {
        stop: None,
        saving: Saving::Failed(SaveFailure::NoDataDirectory),
    };
    assert!(
        said.notice(&recorded)
            .ends_with("ingen datamapp: LOCALAPPDATA är inte satt till en absolut mapp"),
        "{}",
        said.notice(&recorded)
    );
    let elevated = Notice::Refused {
        start: Start::Clicking,
        why: Hindrance::Elevated,
    };
    assert!(
        said.notice(&elevated)
            .ends_with("Auto Clicker kan inte klicka i ett program som körs som administratör"),
        "{}",
        said.notice(&elevated)
    );
    let ctrl_f9 = Binding::parse("Ctrl+F9").unwrap();
    let refused = Problem::HotkeysRefused(vec![
        Unregistered {
            hotkey: ctrl_f9,
            why: NotRegistered::Taken,
        },
        Unregistered {
            hotkey: ctrl_f9,
            why: NotRegistered::Failed(Quoted::new("Access is denied. (os error 5)")),
        },
    ]);
    assert_eq!(
        said.problem(&refused),
        "snabbtangenten Ctrl+F9 fungerar inte: ett annat program använder den; snabbtangenten \
         Ctrl+F9 fungerar inte: Windows nekade den: Access is denied. (os error 5)"
    );
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

/// Why a pick or clicking at a picked point does not go ahead.
fn point_screens(said: &Said<'_>) {
    let note = |note: Note| note_text(said, &note).to_string();
    let refused = RecordingOutcome::NotPlaying(Some(Notice::Refused {
        start: Start::Clicking,
        why: Hindrance::Screens(PointScreens::Changed),
    }));
    assert_eq!(
        note(Note::Recording(refused)),
        "Startade inte: skärmarna har ändrats sedan den fasta punkten valdes: välj den igen \
         i Inställningar"
    );
    let stopped =
        RecordingOutcome::NotPlaying(Some(Notice::Stopped(Stop::Screens(PointScreens::Unknown))));
    assert_eq!(
        note(Note::Recording(stopped)),
        "Klickandet stoppades: skärmlayouten är inte känd, så den valda punkten kan inte \
         kontrolleras"
    );
    assert_eq!(
        note(Note::Unavailable(Unavailability::ScreensChanged)),
        "Inte tillgängligt: skärmarna ändrades medan punkten valdes: välj den igen"
    );
    assert_eq!(
        note(Note::Unavailable(Unavailability::ScreensUnknown(
            NoScreens::Lost
        ))),
        "Inte tillgängligt: skärmlayouten är inte känd: anslutningen till Wayland-displayen \
         bröts"
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

/// Why recordings keep movement only, each reason in Swedish.
fn no_positions(said: &Said<'_>) {
    let reasons = [
        (
            NoPositions::Screens(NoScreens::NoDisplay),
            "programmet startade utan en Wayland-display (WAYLAND_DISPLAY); se README",
        ),
        (
            NoPositions::Screens(NoScreens::CannotConnect),
            "programmet kunde inte ansluta till Wayland-displayen",
        ),
        (
            NoPositions::Screens(NoScreens::NoXdgOutput),
            "skrivbordet berättar inte var skärmarna är (ingen xdg-output)",
        ),
        (
            NoPositions::Screens(NoScreens::Lost),
            "anslutningen till Wayland-displayen bröts",
        ),
        (
            NoPositions::Screens(NoScreens::ProtocolError),
            "Wayland-displayen avvisade programmets förfrågningar",
        ),
        (
            NoPositions::Screens(NoScreens::NotWhole),
            "skrivbordet har inte beskrivit alla skärmar",
        ),
        (
            NoPositions::Pointer(NoPointer::NoSource),
            "inget på det här skrivbordet rapporterar var pekaren är",
        ),
        (
            NoPositions::Pointer(NoPointer::Unusable),
            "KWins pekarskript kan inte användas; se loggen",
        ),
        (
            NoPositions::Pointer(NoPointer::HyprlandNoAnswer),
            "Hyprlands socket svarade inte",
        ),
        (
            NoPositions::Pointer(NoPointer::HyprlandUnreadable),
            "Hyprland svarade i en form som programmet inte kan läsa",
        ),
        (
            NoPositions::Pointer(NoPointer::HyprlandOtherDisplay),
            "Hyprlands socket hör till en annan Wayland-display",
        ),
        (
            NoPositions::Pointer(NoPointer::DisplayNoPosition),
            "Wayland-displayen gav ingen pekarposition; se loggen",
        ),
    ];
    for (why, swedish) in reasons {
        assert_eq!(
            said.problem(&Problem::NoPositions(why)),
            format!("Inspelningar sparar bara rörelser: {swedish}")
        );
    }
}

/// Why the on-screen display cannot show, each reason in Swedish.
fn no_on_screen(said: &Said<'_>) {
    let reasons = [
        (
            NoOnScreen::NoDisplay,
            "programmet startade utan en Wayland-display (WAYLAND_DISPLAY); se README",
        ),
        (
            NoOnScreen::CannotConnect,
            "programmet kunde inte ansluta till Wayland-displayen",
        ),
        (
            NoOnScreen::NoXdgOutput,
            "skrivbordet berättar inte skärmarnas storlek för programmet (inget xdg-output)",
        ),
        (
            NoOnScreen::NoLayerShell,
            "skrivbordet kan inte visa den (inget layer-shell)",
        ),
    ];
    for (why, swedish) in reasons {
        assert_eq!(
            said.problem(&Problem::NoOnScreen(why)),
            format!("Skärmvisningen kan inte visas: {swedish}")
        );
    }
}

/// What Play, Rename and Delete answer.
fn recordings(said: &Said<'_>) {
    let note = |note: Note| note_text(said, &note).to_string();
    let held = Notice::Playing(PlayStart::HeldAndRelative {
        seconds: 2,
        fallback: Fallback::Changed,
    });
    assert_eq!(
        said.notice(&held),
        "Spelar upp fast snabbtangenten fortfarande hölls nere efter 2 s; dess tangenter \
         blandas in i det som spelas upp. Spelar upp med relativa rörelser: en skärm som \
         inspelningen använde har flyttats, bytt storlek eller försvunnit"
    );
    assert_eq!(
        said.notice(&Notice::Playing(PlayStart::Relative(Fallback::Unknown))),
        "Spelar upp med relativa rörelser: skärmlayouten är inte känd, så de inspelade \
         positionerna kan inte användas"
    );
    let not_playing = RecordingOutcome::NotPlaying(Some(Notice::Unplayable {
        recording: Titled::new(
            FileName::parse("2026-11-09_18-04-05.123.json").unwrap(),
            Some("Farm loop"),
        ),
        why: Unplayable::TooShort { ms: 20 },
    }));
    assert_eq!(
        note(Note::Recording(not_playing)),
        "Spelar inte upp Farm loop (2026-11-09 18:04:05): den är kortare än 20 ms, för kort för att spelas mer än en \
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
    let full = RecordingOutcome::NotRenamed(ChangeFailure::NotDone {
        step: Step::Written,
        why: Quoted::new("No space left on device (os error 28)"),
    });
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

/// What a delete of several answers, in Swedish, with its plurals.
fn deletions(said: &Said<'_>) {
    let file =
        |minute: u32| FileName::parse(&format!("2026-11-09_18-{minute:02}-00.000.json")).unwrap();
    let one = Removal {
        deleted: 1,
        ..Removal::default()
    };
    assert_eq!(said.removal(&one), "Raderade 1 inspelning.");
    let unsynced = Removal {
        deleted: 3,
        unsynced: Some(Quoted::new("EIO")),
        ..Removal::default()
    };
    assert_eq!(
        said.removal(&unsynced),
        "Raderade 3 inspelningar, men det kanske inte klarar ett strömavbrott: EIO."
    );
    let failed = |minute, name: Option<&str>, why| NotDeleted {
        recording: Titled::new(file(minute), name),
        why,
    };
    let short = Removal {
        deleted: 2,
        failed: vec![
            failed(1, None, ChangeFailure::Missing),
            failed(
                2,
                Some("Farm loop"),
                ChangeFailure::NotDone {
                    step: Step::Deleted,
                    why: Quoted::new("Permission denied (os error 13)"),
                },
            ),
        ],
        more_failed: 4,
        unsynced: None,
    };
    assert_eq!(
        said.removal(&short),
        "Raderade 2 inspelningar. Kunde inte tas bort: Inspelning 2026-11-09 18:01:00: filen \
         finns inte längre; Farm loop (2026-11-09 18:02:00): filen kunde inte tas bort: \
         Permission denied (os error 13). 4 till kunde inte tas bort."
    );
    // None went: only what did not.
    let none = Removal {
        failed: vec![failed(1, None, ChangeFailure::Missing)],
        ..Removal::default()
    };
    assert_eq!(
        said.removal(&none),
        "Kunde inte tas bort: Inspelning 2026-11-09 18:01:00: filen finns inte längre."
    );
    assert_eq!(
        said.not_deleted(&ChangeFailure::Folder(Quoted::new("EACCES"))),
        "Inte borttagen: EACCES"
    );
    let t = said.texts;
    assert_eq!(
        t.invoke_delete_all_question(1),
        "Radera den enda inspelningen för gott? Det går inte att ångra."
    );
    assert_eq!(t.invoke_more_not_deleted(1), "1 till kunde inte tas bort.");
}

/// A recording named in a notice: generated names in Swedish, the user's
/// as they are.
fn titles(said: &Said<'_>) {
    let saved = Notice::Recorded {
        stop: None,
        saving: Saving::Saved {
            name: Quoted::new("Recording 2026-11-09 18:04"),
            file: Some(FileName::parse("2026-11-09_18-04-05.123.json").unwrap()),
        },
    };
    assert_eq!(
        said.notice(&saved),
        "Sparade \u{201d}Inspelning 2026-11-09 18:04:05\u{201d}"
    );
    let made = || FileName::parse("2026-11-09_18-04-05.123.json").unwrap();
    let finished = Notice::Finished(Some(Titled::new(made(), None)));
    assert_eq!(
        said.notice(&finished),
        "Spelade upp Inspelning 2026-11-09 18:04:05 till slutet"
    );
    let named = Notice::Finished(Some(Titled::new(made(), Some("Recording practice"))));
    assert_eq!(
        said.notice(&named),
        "Spelade upp Recording practice (2026-11-09 18:04:05) till slutet"
    );
}
