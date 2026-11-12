use std::sync::Arc;

use auto_clicker_core::config::Config;
use auto_clicker_core::controller::NoRecording;
use auto_clicker_core::hotkeys::Unavailable;

use super::*;

fn view(activity: Activity) -> View {
    let config = Config::default();
    View {
        activity,
        settings: config.clicker,
        hotkey: Hotkey::Bound(config.hotkeys.toggle_clicking),
        record_hotkey: Hotkey::Bound(config.hotkeys.toggle_recording),
        no_recording: None,
        play_hotkey: Hotkey::Desktop(Some("Meta+P".to_owned())),
        playback: config.playback,
        recordings: Arc::from([]),
        playing: None,
        notice: None,
        problems: Vec::new(),
        routes: auto_clicker_core::controller::Routes::default(),
    }
}

#[test]
fn english_is_as_built_and_swedish_has_every_menu_label() {
    let labels = [
        "Start clicking",
        "Stop clicking",
        "Record",
        "Stop recording",
        "Play",
        "Stop playing",
        "Settings",
        "Quit",
        "No recordings yet",
        NoRecording::DesktopHotkeys.label(),
        NoRecording::Keyboards(Unavailable::NoPermission).label(),
        NoRecording::Keyboards(Unavailable::NoKeyboard).label(),
    ];
    for english in labels {
        assert_eq!(label(english, Language::English), english);
        assert_ne!(label(english, Language::Swedish), english, "{english}");
    }
    assert_eq!(
        label("Recording 2026-11-09 18:04:05", Language::Swedish),
        "Inspelning 2026-11-09 18:04:05"
    );
}

#[test]
fn the_title_line_speaks_the_language() {
    let waiting = view(Activity::WaitingForRelease);
    assert_eq!(title(&waiting, Language::English), waiting.title());
    assert_eq!(
        title(&waiting, Language::Swedish),
        "Autoklickare: börjar när du släpper Ctrl och Shift"
    );
    let pause = DESKTOP_RELEASE_PAUSE.as_secs();
    assert_eq!(
        title(&view(Activity::PlayWaitingForRelease), Language::Swedish),
        format!("Autoklickare: spelar upp {pause} s efter att du släppt snabbtangenten")
    );
    let every = [
        Activity::Idle,
        Activity::StartingIn(3),
        Activity::Opening,
        Activity::Clicking,
        Activity::RecordingIn(2),
        Activity::Recording,
        Activity::PlayingIn(1),
        Activity::PlayOpening,
        Activity::Playing,
    ];
    for activity in every {
        let shown = title(&view(activity), Language::Swedish);
        assert!(shown.starts_with("Autoklickare: "), "{shown}");
        assert_ne!(shown, view(activity).title());
    }
}

mod messages {
    use std::collections::HashSet;
    use std::mem::discriminant;

    use auto_clicker_core::controller::{Blocked, NoRecording, Routes};
    use auto_clicker_core::hotkeys::{Binding, Refusal, Unavailable};
    use auto_clicker_core::input::Key;
    use auto_clicker_core::message::{
        ConfigProblem, Hindrance, InputFailure, Notice, PlayStart, Problem, Quoted, SaveFailure,
        Saving, Start, Stop, Unplayable,
    };
    use auto_clicker_core::playback::Fallback;

    use super::super::{notice, problem};
    use super::*;

    /// Text from outside the program, which must reach the user as it is.
    const QUOTED: &str = "«quoted: Permission denied (os error 13)»";

    fn quoted() -> Quoted {
        Quoted::new(QUOTED)
    }

    fn key(name: &str) -> Key {
        Key::from_name(name).unwrap()
    }

    fn blocked() -> Vec<Blocked> {
        vec![
            Blocked::Locked,
            Blocked::Inactive,
            Blocked::Sleeping,
            Blocked::Unknown(QUOTED.to_owned().into()),
        ]
    }

    fn inputs() -> Vec<InputFailure> {
        vec![
            InputFailure::NoPermission(quoted()),
            InputFailure::UinputMissing(quoted()),
            InputFailure::CreateDevice(quoted()),
            InputFailure::Write(quoted()),
            InputFailure::PortalUnavailable(quoted()),
            InputFailure::NotAllowed,
            InputFailure::AllowedNow,
            InputFailure::Portal(quoted()),
            InputFailure::SessionLost(quoted()),
        ]
    }

    fn unavailables() -> Vec<Unavailable> {
        vec![
            Unavailable::NoKeyboard,
            Unavailable::NoPermission,
            Unavailable::ReadFailed(QUOTED.to_owned().into()),
            Unavailable::WatchFailed(QUOTED.to_owned().into()),
            Unavailable::Desktop(QUOTED.to_owned().into()),
        ]
    }

    fn refusals() -> Vec<Refusal> {
        vec![
            Refusal::Modifier(key("leftshift")),
            Refusal::HotkeyKey {
                key: key("f6"),
                trigger: "Meta+F6".to_owned(),
            },
            Refusal::Unreadable("Meta+P".to_owned()),
            Refusal::NotReported,
        ]
    }

    fn problems() -> Vec<Problem> {
        let mut all = vec![
            Problem::Config(ConfigProblem::NoDirectory),
            Problem::Config(ConfigProblem::Unreadable {
                file: Quoted::new("/home/me/config.toml"),
                why: quoted(),
            }),
            Problem::Config(ConfigProblem::Invalid {
                file: Quoted::new("/home/me/config.toml"),
                line: Some(4),
                why: quoted(),
            }),
            Problem::Config(ConfigProblem::Invalid {
                file: Quoted::new("/home/me/config.toml"),
                line: None,
                why: quoted(),
            }),
            Problem::Unlisted(quoted()),
        ];
        all.extend(unavailables().into_iter().map(Problem::Hotkeys));
        all.extend(blocked().into_iter().map(Problem::Blocked));
        all.extend(inputs().into_iter().map(Problem::Input));
        all
    }

    fn hindrances() -> Vec<Hindrance> {
        let mut all = vec![
            Hindrance::HeldTooLong { seconds: 30 },
            Hindrance::ReleaseLost(Binding::parse("Ctrl+Shift+F6").unwrap()),
            Hindrance::HotkeysStopped,
            Hindrance::ClickingOn,
            Hindrance::Unlisted(quoted()),
            Hindrance::NoRecording(NoRecording::DesktopHotkeys),
            Hindrance::NoClock,
        ];
        all.extend(blocked().into_iter().map(Hindrance::Blocked));
        all.extend(refusals().into_iter().map(Hindrance::OwnKey));
        all.extend(inputs().into_iter().map(Hindrance::Input));
        all.extend(unavailables().into_iter().map(Hindrance::Hotkeys));
        all.extend(
            unavailables()
                .into_iter()
                .map(|why| Hindrance::NoRecording(NoRecording::Keyboards(why))),
        );
        all
    }

    fn unplayables() -> Vec<Unplayable> {
        let mut all = vec![
            Unplayable::Missing,
            Unplayable::Unreadable(quoted()),
            Unplayable::TooLarge { mib: 256 },
            Unplayable::Invalid(quoted()),
            Unplayable::TooShort { ms: 20 },
        ];
        all.extend(refusals().into_iter().map(Unplayable::PressesHotkeyKey));
        all
    }

    fn stops() -> Vec<Stop> {
        let mut all = vec![
            Stop::LayoutChanged,
            Stop::LayoutUnknown,
            Stop::Full,
            Stop::Limit { minutes: 60 },
        ];
        all.extend(blocked().into_iter().map(Stop::Blocked));
        all.extend(refusals().into_iter().map(Stop::OwnKey));
        all.extend(unavailables().into_iter().map(Stop::Hotkeys));
        all
    }

    fn savings() -> Vec<Saving> {
        vec![
            Saving::Nothing,
            Saving::Saved("Recording 2026-11-09 18:04".to_owned()),
            Saving::Failed(SaveFailure::NoDataDirectory),
            Saving::Failed(SaveFailure::Invalid(quoted())),
            Saving::Failed(SaveFailure::Write(quoted())),
        ]
    }

    fn notices() -> Vec<Notice> {
        let recording = || "Recording 2026-11-09 18:04:05".to_owned();
        let mut all = vec![
            Notice::NothingToPlay,
            Notice::Finished(Some(recording())),
            Notice::Finished(None),
            Notice::Playing(PlayStart::Held { seconds: 2 }),
            Notice::Playing(PlayStart::Relative(Fallback::Unknown)),
            Notice::Playing(PlayStart::Relative(Fallback::Changed)),
            Notice::Playing(PlayStart::HeldAndRelative {
                seconds: 2,
                fallback: Fallback::Changed,
            }),
        ];
        for start in [Start::Clicking, Start::Playback, Start::Recording] {
            all.extend(
                hindrances()
                    .into_iter()
                    .map(|why| Notice::Refused { start, why }),
            );
        }
        all.extend(unplayables().into_iter().map(|why| Notice::Unplayable {
            recording: recording(),
            why,
        }));
        all.extend(stops().into_iter().map(Notice::Stopped));
        for playback in [false, true] {
            all.extend(
                inputs()
                    .into_iter()
                    .map(|why| Notice::RunFailed { playback, why }),
            );
        }
        for saving in savings() {
            all.push(Notice::Recorded {
                stop: None,
                saving: saving.clone(),
            });
            all.extend(stops().into_iter().map(|stop| Notice::Recorded {
                stop: Some(stop),
                saving: saving.clone(),
            }));
        }
        all
    }

    /// Asserts each of `english` and `swedish` differ, and that quoted text
    /// reaches the Swedish as it is.
    fn assert_swedish(english: &str, swedish: &str) {
        assert_ne!(swedish, english);
        assert!(!swedish.is_empty(), "{english}");
        assert_eq!(
            swedish.contains(QUOTED),
            english.contains(QUOTED),
            "{english}\n{swedish}"
        );
    }

    /// How many of `variants` the values cover, by kind.
    fn kinds<T>(values: &[T]) -> usize {
        values
            .iter()
            .map(discriminant)
            .collect::<HashSet<_>>()
            .len()
    }

    #[test]
    fn every_notice_has_a_swedish_form_that_keeps_quoted_text() {
        let all = notices();
        // A sample of each kind of notice and of each reason inside one.
        assert_eq!(kinds(&all), 8);
        assert_eq!(kinds(&hindrances()), 11);
        assert_eq!(kinds(&unplayables()), 6);
        assert_eq!(kinds(&stops()), 7);
        assert_eq!(kinds(&savings()), 3);
        for notice in all {
            assert_swedish(&notice.to_string(), &notice::swedish(&notice));
        }
    }

    #[test]
    fn every_problem_has_a_swedish_form_that_keeps_quoted_text() {
        let all = problems();
        assert_eq!(kinds(&all), 5);
        assert_eq!(kinds(&blocked()), 4);
        assert_eq!(kinds(&inputs()), 9);
        assert_eq!(kinds(&unavailables()), 5);
        assert_eq!(kinds(&refusals()), 4);
        for problem in all {
            assert_swedish(&problem.to_string(), &problem::swedish(&problem));
        }
    }

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
        let finished = Notice::Finished(Some("Recording 2026-11-09 18:04:05".to_owned()));
        assert_eq!(
            notice::swedish(&finished),
            "Spelade upp Inspelning 2026-11-09 18:04:05 till slutet"
        );
        let saved = Notice::Recorded {
            stop: Some(Stop::Limit { minutes: 60 }),
            saving: Saving::Saved("Recording 2026-11-09 18:04".to_owned()),
        };
        assert_eq!(
            notice::swedish(&saved),
            "Inspelningen stoppades vid sin gräns på 60 minuter. Sparade \
             \u{201d}Recording 2026-11-09 18:04\u{201d}"
        );
    }

    #[test]
    fn the_tooltip_speaks_the_language_line_by_line() {
        let mut shown = view(Activity::Recording);
        shown.routes = Routes::default();
        shown.playing = Some("Recording 2026-11-09 18:04:05".to_owned());
        shown.notice = Some(Notice::NothingToPlay);
        shown.problems = vec![Problem::Blocked(Blocked::Locked)];
        assert_eq!(details(&shown, Language::English), shown.details());
        assert_eq!(
            details(&shown, Language::Swedish),
            [
                "Klickar med vänsterknappen var 100 ms",
                "Ctrl+Shift+F6 startar och stoppar klickandet",
                "Ctrl+Shift+F7 startar inspelningen, och stoppar och sparar den",
                "Meta+P spelar upp den senaste inspelningen en gång, och stoppar den",
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
                    trigger: "Meta+F6".to_owned(),
                }))),
                "Stoppat: f6 är tangenten i snabbtangenten Meta+F6, och skrivbordet kan inte \
                 skilja programmets tangenttryck från dina",
            ),
            (
                notice::swedish(&Notice::Stopped(Stop::OwnKey(Refusal::Unreadable(
                    "Meta+P".to_owned(),
                )))),
                "Stoppat: snabbtangenten \u{201d}Meta+P\u{201d} ligger inte på en \
                 funktionstangent (F1 till F24), så programmet kan inte avgöra om dess egna \
                 tangenttryck skulle utlösa den; ändra den i Systeminställningar",
            ),
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
                "Ingen snabbtangent spelar upp den senaste inspelningen 3 gånger i 1,5× \
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
            "Meta+P spelar upp den senaste inspelningen tills den stoppas, och stoppar den"
        );
    }
}
