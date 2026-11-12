//! One value of each kind of message, for the Swedish tests.

use auto_clicker_core::controller::{Blocked, NoRecording};
use auto_clicker_core::hotkeys::{Binding, Refusal, Unavailable};
use auto_clicker_core::input::Key;
use auto_clicker_core::message::{
    ConfigProblem, Hindrance, InputFailure, Notice, PlayStart, PointScreens, Problem, Quoted,
    SaveFailure, Saving, Start, Stop, Titled, Unplayable,
};
use auto_clicker_core::playback::Fallback;
use auto_clicker_core::recording::FileName;

/// Text from outside the program, which must reach the user as it is.
pub(super) const QUOTED: &str = "«quoted: Permission denied (os error 13)»";

pub(super) fn quoted() -> Quoted {
    Quoted::new(QUOTED)
}

/// The recording made at 18:04:05, named `name` in its header.
pub(super) fn generated(name: Option<&str>) -> Titled {
    let file = FileName::parse("2026-11-09_18-04-05.123.json").unwrap();
    Titled::new(file, name)
}

pub(super) fn key(name: &str) -> Key {
    Key::from_name(name).unwrap()
}

pub(super) fn blocked() -> Vec<Blocked> {
    vec![
        Blocked::Locked,
        Blocked::Inactive,
        Blocked::Sleeping,
        Blocked::Unknown(QUOTED.to_owned().into()),
    ]
}

pub(super) fn inputs() -> Vec<InputFailure> {
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

pub(super) fn unavailables() -> Vec<Unavailable> {
    vec![
        Unavailable::NoKeyboard,
        Unavailable::NoPermission,
        Unavailable::ReadFailed(QUOTED.to_owned().into()),
        Unavailable::WatchFailed(QUOTED.to_owned().into()),
        Unavailable::Desktop(QUOTED.to_owned().into()),
    ]
}

pub(super) fn refusals() -> Vec<Refusal> {
    vec![
        Refusal::Modifier(key("leftshift")),
        Refusal::HotkeyKey {
            key: key("f6"),
            trigger: "Meta+F6".to_owned().into(),
        },
        Refusal::Unreadable("Meta+P".to_owned().into()),
        Refusal::NotReported,
    ]
}

pub(super) fn problems() -> Vec<Problem> {
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
        Problem::CurrentUnread(quoted()),
        Problem::CurrentUnkept(quoted()),
    ];
    all.extend(unavailables().into_iter().map(Problem::Hotkeys));
    all.extend(blocked().into_iter().map(Problem::Blocked));
    all.extend(inputs().into_iter().map(Problem::Input));
    all
}

pub(super) fn hindrances() -> Vec<Hindrance> {
    let mut all = vec![
        Hindrance::HeldTooLong { seconds: 30 },
        Hindrance::ReleaseLost(Binding::parse("Ctrl+Shift+F6").unwrap()),
        Hindrance::HotkeysStopped,
        Hindrance::ClickingOn,
        Hindrance::Unlisted(quoted()),
        Hindrance::NoRecording(NoRecording::DesktopHotkeys),
        Hindrance::NoClock,
        Hindrance::Screens(PointScreens::Changed),
        Hindrance::Screens(PointScreens::Unknown),
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

pub(super) fn unplayables() -> Vec<Unplayable> {
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

pub(super) fn stops() -> Vec<Stop> {
    let mut all = vec![
        Stop::LayoutChanged,
        Stop::LayoutUnknown,
        Stop::Screens(PointScreens::Changed),
        Stop::Screens(PointScreens::Unknown),
        Stop::Full,
        Stop::Limit { minutes: 60 },
    ];
    all.extend(blocked().into_iter().map(Stop::Blocked));
    all.extend(refusals().into_iter().map(Stop::OwnKey));
    all.extend(unavailables().into_iter().map(Stop::Hotkeys));
    all
}

pub(super) fn savings() -> Vec<Saving> {
    vec![
        Saving::Nothing,
        Saving::Saved {
            name: Quoted::new("Recording 2026-11-09 18:04"),
            file: Some(generated(None).file),
        },
        Saving::Failed(SaveFailure::NoDataDirectory),
        Saving::Failed(SaveFailure::Invalid(quoted())),
        Saving::Failed(SaveFailure::Write(quoted())),
    ]
}

pub(super) fn notices() -> Vec<Notice> {
    let recording = || generated(None);
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
