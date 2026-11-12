//! The Swedish as written: one message of each family, and the tooltip
//! lines.

use auto_clicker_core::controller::{Blocked, NoRecording, Routes};
use auto_clicker_core::hotkeys::{Refusal, Unavailable};
use auto_clicker_core::input::Key;
use auto_clicker_core::message::{
    ConfigProblem, Hindrance, InputFailure, Notice, PlayStart, Problem, Quoted, Saving, Start, Stop,
};
use auto_clicker_core::playback::Fallback;

use super::super::{details, notice, problem};
use super::samples::*;
use super::view;
use crate::tray_text::Language;
use auto_clicker_core::controller::Activity;

#[test]
fn quoted_text_is_kept_inside_the_swedish_sentence() {
    let failed = Notice::RunFailed {
        playback: true,
        why: InputFailure::SessionLost(Quoted::new("org.freedesktop.portal: closed")),
    };
    assert_eq!(
        notice::swedish(&failed),
        "Uppspelningen stoppades: skrivbordet avslutade fjärrinmatningen: \
         org.freedesktop.portal: closed"
    );
    let invalid = Problem::Config(ConfigProblem::Invalid {
        file: Quoted::new("/home/me/config.toml"),
        line: Some(4),
        why: Quoted::new("expected a boolean"),
    });
    assert_eq!(
        problem::swedish(&invalid),
        "/home/me/config.toml, rad 4: expected a boolean"
    );
}

#[test]
fn recordings_are_named_as_the_swedish_menu_names_them() {
    let finished = Notice::Finished(Some(generated(Some("Recording 2026-11-09 18:04"))));
    assert_eq!(
        notice::swedish(&finished),
        "Spelade upp Inspelning 2026-11-09 18:04:05 till slutet"
    );
    // A name the user gave stays as it is, "Recording" and all, and so
    // does a saved name whose file is not known.
    let unknown = Notice::Recorded {
        stop: None,
        saving: Saving::Saved {
            name: Quoted::new("Recording 2026-11-09 18:04"),
            file: None,
        },
    };
    assert_eq!(
        notice::swedish(&unknown),
        "Sparade \u{201d}Recording 2026-11-09 18:04\u{201d}"
    );
    // A name the user gave stays as it is, "Recording" and all.
    let named = Notice::Finished(Some(generated(Some("Recording practice"))));
    assert_eq!(
        notice::swedish(&named),
        "Spelade upp Recording practice (2026-11-09 18:04:05) till slutet"
    );
    let saved = Notice::Recorded {
        stop: Some(Stop::Limit { minutes: 60 }),
        saving: Saving::Saved {
            name: Quoted::new("Recording 2026-11-09 18:04"),
            file: Some(generated(None).file),
        },
    };
    assert_eq!(
        notice::swedish(&saved),
        "Inspelningen stoppades vid sin gräns på 60 minuter. Sparade \
         \u{201d}Inspelning 2026-11-09 18:04:05\u{201d}"
    );
}

#[test]
fn the_tooltip_speaks_the_language_line_by_line() {
    let mut shown = view(Activity::Recording);
    shown.routes = Routes::default();
    shown.playing = Some(generated(None));
    shown.notice = Some(Notice::NothingToPlay);
    shown.problems = vec![Problem::Blocked(Blocked::Locked)];
    assert_eq!(details(&shown, Language::English), shown.details());
    assert_eq!(
        details(&shown, Language::Swedish),
        [
            "Klickar med vänsterknappen var 100 ms",
            "Ctrl+Shift+F6 startar och stoppar klickandet",
            "Ctrl+Shift+F7 startar inspelningen, och stoppar och sparar den",
            "Meta+P spelar upp den aktuella inspelningen en gång, och stoppar den",
            "Klickar via /dev/uinput, snabbtangenter från tangentborden",
            "En inspelning stoppas av sig själv efter 60 minuter",
            "Spelar upp Inspelning 2026-11-09 18:04:05",
            "Inget att spela upp: inga inspelningar än",
            "Kan inte börja klicka: skärmen är låst",
        ]
    );
}

#[test]
fn swedish_reads_as_written() {
    let modifier = Refusal::Modifier(key("leftshift"));
    let rows = [
        (
            notice::swedish(&Notice::Refused {
                start: Start::Clicking,
                why: Hindrance::OwnKey(modifier.clone()),
            }),
            "Startade inte: leftshift är en modifierare, och om programmet håller den \
             ändras vilken snabbtangent skrivbordet ser",
        ),
        (
            notice::swedish(&Notice::Refused {
                start: Start::Playback,
                why: Hindrance::ClickingOn,
            }),
            "Spelar inte upp: klickandet är på; stoppa det först",
        ),
        (
            notice::swedish(&Notice::Refused {
                start: Start::Recording,
                why: Hindrance::NoRecording(NoRecording::DesktopHotkeys),
            }),
            "Spelar inte in: inspelning går inte med skrivbordets genvägar",
        ),
        (
            notice::swedish(&Notice::Playing(PlayStart::HeldAndRelative {
                seconds: 2,
                fallback: Fallback::Unknown,
            })),
            "Spelar upp fast snabbtangenten fortfarande hölls nere efter 2 s; dess tangenter \
             blandas in i det som spelas upp. Spelar upp med relativa rörelser: \
             skärmlayouten är inte känd, så de inspelade positionerna kan inte användas",
        ),
        (
            notice::swedish(&Notice::Playing(PlayStart::Relative(Fallback::Changed))),
            "Spelar upp med relativa rörelser: en skärm som inspelningen använde har \
             flyttats, bytt storlek eller försvunnit",
        ),
        (
            notice::swedish(&Notice::Stopped(Stop::OwnKey(Refusal::HotkeyKey {
                key: key("f6"),
                trigger: "Meta+F6".to_owned().into(),
            }))),
            "Stoppat: f6 är tangenten i snabbtangenten Meta+F6, och skrivbordet kan inte \
             skilja programmets tangenttryck från dina",
        ),
        (
            notice::swedish(&Notice::Stopped(Stop::OwnKey(Refusal::Unreadable(
                "Meta+P".to_owned().into(),
            )))),
            "Stoppat: snabbtangenten \u{201d}Meta+P\u{201d} ligger inte på en \
             funktionstangent (F1 till F24), så programmet kan inte avgöra om dess egna \
             tangenttryck skulle utlösa den; ändra den i Systeminställningar",
        ),
    ];
    for (swedish, expected) in rows {
        assert_eq!(swedish, expected);
    }
}

#[test]
fn swedish_problems_read_as_written() {
    let rows = [
        (
            problem::swedish(&Problem::Hotkeys(Unavailable::NoPermission)),
            "snabbtangenterna är inte tillgängliga: tangentborden kan inte läsas; lägg till \
             din användare i gruppen input och logga in igen, eller sätt hotkeys = \
             \"portal\" under [input] för skrivbordets genvägar",
        ),
        (
            problem::swedish(&Problem::Input(InputFailure::NotAllowed)),
            "Kan inte börja klicka: inte tillåtet i skrivbordets dialog; nästa start frågar \
             igen",
        ),
    ];
    for (swedish, expected) in rows {
        assert_eq!(swedish, expected);
    }
}

#[test]
fn swedish_tooltip_lines_read_as_written() {
    use auto_clicker_core::clicker::{Interval, Jitter, Mode, Position, Repeat};
    use auto_clicker_core::controller::Hotkey;
    use auto_clicker_core::input::{AbsolutePosition, Button};
    use auto_clicker_core::playback::Speed;
    use std::num::NonZeroU32;
    use std::time::Duration;

    let mut shown = view(Activity::Idle);
    shown.settings.interval = Interval::new(Duration::from_secs(2)).unwrap();
    shown.settings.jitter = Jitter::new(10).unwrap();
    shown.settings.button = Button::Side;
    shown.settings.repeat = Repeat::Times(NonZeroU32::new(50).unwrap());
    shown.settings.position = Position::Fixed(AbsolutePosition { x: 1, y: 2 });
    shown.play_hotkey = Hotkey::Desktop(Some(String::new()));
    shown.playback.repeat = Repeat::Times(NonZeroU32::new(3).unwrap());
    shown.playback.speed = Speed::from_ratio(1.5).unwrap();
    shown.no_recording = Some(NoRecording::DesktopHotkeys);
    let lines = details(&shown, Language::Swedish);
    assert_eq!(
        lines[..4],
        [
            "Klickar med bakåtknappen var 2 s (±10 %), 50 gånger på en fast punkt",
            "Ctrl+Shift+F6 startar och stoppar klickandet",
            "Inspelning går inte med skrivbordets genvägar: inget läser tangentborden och \
             mössen. Den kräver gruppen input, och hotkeys = \"auto\" eller \"kernel\" \
             under [input] i konfigurationen.",
            "Ingen snabbtangent spelar upp den aktuella inspelningen 3 gånger i 1,5× \
             hastighet, och stoppar den: välj en i Systeminställningar, under Genvägar",
        ]
    );
    let modes = [
        (
            Mode::DoubleClick,
            "Dubbelklickar med vänsterknappen var 100 ms",
        ),
        (Mode::HoldButton, "Håller vänsterknappen nedtryckt"),
        (Mode::KeyRepeat, "Trycker på space var 100 ms"),
        (Mode::HoldKey, "Håller space nedtryckt"),
    ];
    for (mode, line) in modes {
        let mut shown = view(Activity::Idle);
        shown.settings.mode = mode;
        shown.settings.position = Position::Fixed(AbsolutePosition { x: 1, y: 2 });
        let expected = match mode {
            Mode::DoubleClick | Mode::HoldButton => format!("{line} på en fast punkt"),
            _ => line.to_owned(),
        };
        assert_eq!(details(&shown, Language::Swedish)[0], expected, "{mode:?}");
    }
    let mut unnamed = view(Activity::Idle);
    unnamed.settings.mode = Mode::HoldKey;
    unnamed.settings.key = Key::new(248).unwrap();
    assert_eq!(
        details(&unnamed, Language::Swedish)[0],
        "Håller tangent 248 nedtryckt"
    );
    let mut forever = view(Activity::Idle);
    forever.playback.repeat = Repeat::UntilStopped;
    assert_eq!(
        details(&forever, Language::Swedish)[3],
        "Meta+P spelar upp den aktuella inspelningen tills den stoppas, och stoppar den"
    );
}
