//! Each message's English, as the program wrote it before messages were
//! values.

use super::*;
use crate::config::Reason;
use crate::controller::NoRecording;
use crate::hotkeys::{Binding, Refusal};
use crate::input::Key;
use crate::playback::Fallback;

/// A recording the user named "R".
fn r() -> Titled {
    let file = crate::recording::FileName::parse("2026-11-09_18-04-05.123.json").unwrap();
    Titled::new(file, Some("R"))
}

fn q() -> Quoted {
    Quoted::new("why")
}

fn assert_english<T: fmt::Display>(cases: &[(T, &str)]) {
    for (value, english) in cases {
        assert_eq!(value.to_string(), *english);
    }
}

#[test]
fn problems_read_as_before() {
    let file = || Quoted::new("/c/config.toml");
    assert_english(&[
        (
            Problem::Config(ConfigProblem::NoDirectory),
            "no config directory: neither XDG_CONFIG_HOME nor HOME is set",
        ),
        (
            Problem::Config(ConfigProblem::Unreadable {
                file: file(),
                why: q(),
            }),
            "/c/config.toml: could not read the file: why",
        ),
        (
            Problem::Config(ConfigProblem::Invalid {
                file: file(),
                line: Some(3),
                why: q(),
            }),
            "/c/config.toml:3: why",
        ),
        (
            Problem::Config(ConfigProblem::Invalid {
                file: file(),
                line: None,
                why: q(),
            }),
            "/c/config.toml: why",
        ),
        (
            Problem::Hotkeys(Unavailable::NoKeyboard),
            "hotkeys are unavailable: no keyboard found",
        ),
        (
            Problem::Blocked(Blocked::Inactive),
            "Clicking cannot start: another session is in front",
        ),
        (
            Problem::Input(InputFailure::NotAllowed),
            "Clicking cannot start: not allowed in the desktop's dialog; Start asks again",
        ),
        (
            Problem::Unlisted(q()),
            "The recordings cannot be listed: why",
        ),
    ]);
}

#[test]
fn input_failures_read_as_before() {
    assert_english(&[
        (
            InputFailure::NoPermission(q()),
            "no permission to open /dev/uinput (why); install the package's udev rule and \
             log in again, or add your user to the `input` group",
        ),
        (
            InputFailure::UinputMissing(q()),
            "/dev/uinput is not available (why); load the kernel module with `sudo modprobe \
             uinput`",
        ),
        (
            InputFailure::CreateDevice(q()),
            "could not create a virtual input device: why",
        ),
        (
            InputFailure::Write(q()),
            "could not write to a virtual input device: why",
        ),
        (
            InputFailure::PortalUnavailable(q()),
            "the desktop's remote input portal cannot be used: why",
        ),
        (InputFailure::AllowedNow, "allowed; press Start again"),
        (
            InputFailure::Portal(q()),
            "the remote input portal failed: why",
        ),
        (
            InputFailure::SessionLost(q()),
            "the desktop ended remote input: why",
        ),
    ]);
}

#[test]
fn answers_read_as_before() {
    assert_english(&[
        (
            Failure::NoConfigDirectory,
            "there is no config directory: neither XDG_CONFIG_HOME nor HOME is set",
        ),
        (
            Failure::Write(q()),
            "the config file could not be written: why",
        ),
        (Failure::BadToken, "the activation token is not one"),
        (Failure::Desktop(q()), "why"),
        (Failure::NotOpened, "the desktop did not open its settings"),
        (
            Failure::Hotkeys(Unavailable::Desktop("gone".to_owned().into())),
            "hotkeys are unavailable: gone",
        ),
    ]);
    assert_english(&[
        (
            Unavailability::DesktopDelivers,
            "the desktop delivers the hotkeys: change them in System Settings, under Shortcuts",
        ),
        (
            Unavailability::Blocked(Blocked::Locked),
            "the screen is locked",
        ),
        (
            Unavailability::ReadsKeyboards,
            "the program reads the keyboards: set the hotkeys in this window",
        ),
        (
            Unavailability::NoShortcuts,
            "the desktop's shortcuts are not available",
        ),
        (
            Unavailability::Hotkeys(Unavailable::NoKeyboard),
            "hotkeys are unavailable: no keyboard found",
        ),
    ]);
}

#[test]
fn reasons_read_as_before() {
    let hotkey = || Binding::parse("Ctrl+F6").unwrap();
    assert_english(&[
        (
            Reason::TrayStartDelay { max: 3600 },
            "tray_start_delay must be from 0 to 3600 seconds",
        ),
        (
            Reason::Interval {
                floor: 10,
                max: 86_400_000,
                double: false,
            },
            "interval_ms must be from 10 to 86400000",
        ),
        (
            Reason::Interval {
                floor: 20,
                max: 86_400_000,
                double: true,
            },
            "interval_ms must be from 20 to 86400000 for a double click, which fits two \
             clicks in each interval",
        ),
        (
            Reason::Rate {
                floor: 10,
                max: 86_400_000,
                double: false,
            },
            "clicks_per_second must be from 0.000012 to 100",
        ),
        (
            Reason::Jitter { max: 50 },
            "jitter_percent must be from 0 to 50",
        ),
        (Reason::Speed, "speed must be from 0.25 to 4"),
        (
            Reason::KeyCode { code: 0 },
            "key code 0 is not from 1 to 248",
        ),
        (
            Reason::Count { count: 0 },
            "repeat 0 is not from 1 to 4294967295; use \"until_stopped\" to repeat until \
             stopped",
        ),
        (
            Reason::BoundTwice(hotkey()),
            "hotkey Ctrl+F6 is bound to two actions",
        ),
        (
            Reason::OnClickerKey(hotkey()),
            "hotkey Ctrl+F6 uses the key the clicker presses; choose another key",
        ),
        (Reason::Binding(q()), "why"),
        (
            Reason::DesktopChooses,
            "the desktop delivers the hotkeys: change them in System Settings, under Shortcuts",
        ),
    ]);
}

#[test]
fn notices_read_as_before() {
    let refused = |start, why| Notice::Refused { start, why };
    let held = Hindrance::HeldTooLong { seconds: 30 };
    assert_english(&[
        (
            refused(Start::Clicking, held),
            "Not started: the hotkey was held for 30 s; let go of it to start",
        ),
        (
            refused(
                Start::Playback,
                Hindrance::ReleaseLost(Binding::parse("Ctrl+F8").unwrap()),
            ),
            "Not playing: could not tell when the hotkey was let go; press Ctrl+F8 again",
        ),
        (
            refused(Start::Recording, Hindrance::HotkeysStopped),
            "Not recording: hotkeys stopped working",
        ),
        (
            refused(Start::Recording, Hindrance::ClickingOn),
            "Not recording: clicking is on; stop it first",
        ),
        (
            refused(Start::Playback, Hindrance::Unlisted(q())),
            "Not playing: the recordings cannot be listed: why",
        ),
        (
            refused(
                Start::Recording,
                Hindrance::NoRecording(NoRecording::DesktopHotkeys),
            ),
            "Not recording: recording is unavailable with the desktop's shortcuts",
        ),
        (
            refused(Start::Recording, Hindrance::NoClock),
            "Not recording: the clock could not be read",
        ),
        (
            refused(Start::Clicking, Hindrance::OwnKey(Refusal::NotReported)),
            "Not started: the desktop has not said yet which keys the hotkeys use, and the \
             program must not press one of them",
        ),
        (Notice::NothingToPlay, "Nothing to play: no recordings yet"),
        (
            Notice::RunFailed {
                playback: false,
                why: InputFailure::Write(q()),
            },
            "Clicking stopped: could not write to a virtual input device: why",
        ),
        (
            Notice::RunFailed {
                playback: true,
                why: InputFailure::Write(q()),
            },
            "Playback stopped: could not write to a virtual input device: why",
        ),
        (
            Notice::Finished(Some(r())),
            "Finished playing R (2026-11-09 18:04:05)",
        ),
        (Notice::Finished(None), "Finished every action"),
    ]);
}

#[test]
fn unplayables_read_as_before() {
    let unplayable = |why| Notice::Unplayable {
        recording: r(),
        why,
    };
    let modifier = Refusal::Modifier(Key::from_name("leftshift").unwrap());
    assert_english(&[
        (
            unplayable(Unplayable::Missing),
            "Not playing R (2026-11-09 18:04:05): the file is no longer there",
        ),
        (
            unplayable(Unplayable::Unreadable(q())),
            "Not playing R (2026-11-09 18:04:05): the file could not be read: why",
        ),
        (
            unplayable(Unplayable::TooLarge { mib: 256 }),
            "Not playing R (2026-11-09 18:04:05): the file is larger than 256 MiB",
        ),
        (
            unplayable(Unplayable::Invalid(q())),
            "Not playing R (2026-11-09 18:04:05): why",
        ),
        (
            unplayable(Unplayable::TooShort { ms: 20 }),
            "Not playing R (2026-11-09 18:04:05): it is shorter than 20 ms, too short to play more than once",
        ),
        (
            unplayable(Unplayable::PressesHotkeyKey(modifier.clone())),
            &format!("Not playing R (2026-11-09 18:04:05): it presses a hotkey's key: {modifier}"),
        ),
    ]);
}

#[test]
fn stops_read_as_before() {
    let modifier = Refusal::Modifier(Key::from_name("leftshift").unwrap());
    assert_english(&[
        (
            Stop::Blocked(Blocked::Sleeping),
            "Stopped: the computer is going to sleep",
        ),
        (
            Stop::OwnKey(modifier.clone()),
            &format!("Stopped: {modifier}"),
        ),
        (
            Stop::LayoutChanged,
            "Playback stopped: the screen layout changed",
        ),
        (
            Stop::LayoutUnknown,
            "Playback stopped: the screen layout is no longer known",
        ),
        (
            Stop::Hotkeys(Unavailable::NoKeyboard),
            "Recording stopped: hotkeys are unavailable: no keyboard found",
        ),
        (
            Stop::Full,
            "Recording stopped: it reached the largest size a recording may have",
        ),
        (
            Stop::Limit { minutes: 60 },
            "Recording stopped at its 60-minute limit",
        ),
    ]);
}

#[test]
fn recordings_ended_read_as_before() {
    let recorded = |stop, saving| Notice::Recorded { stop, saving };
    assert_english(&[
        (
            recorded(None, Saving::Nothing),
            "Nothing was recorded, so nothing was saved",
        ),
        (
            recorded(
                Some(Stop::Full),
                Saving::Saved {
                    name: Quoted::new("R"),
                    file: None,
                },
            ),
            "Recording stopped: it reached the largest size a recording may have. Saved \
             \u{201c}R\u{201d}",
        ),
        (
            recorded(None, Saving::Failed(SaveFailure::NoDataDirectory)),
            "The recording could not be saved: no data directory: neither XDG_DATA_HOME nor \
             HOME is set",
        ),
        (
            recorded(None, Saving::Failed(SaveFailure::Invalid(q()))),
            "The recording could not be saved: why",
        ),
        (
            recorded(None, Saving::Failed(SaveFailure::Write(q()))),
            "The recording could not be saved: why",
        ),
    ]);
}

#[test]
fn a_playback_that_starts_otherwise_says_how() {
    let held = "Playing although the hotkey was still held after 2 s; its keys mix into what \
                plays";
    let relative = Fallback::Unknown.to_string();
    assert_eq!(PlayStart::new(None, None), None);
    assert_english(&[
        (PlayStart::new(Some(2), None).unwrap(), held),
        (
            PlayStart::new(None, Some(Fallback::Unknown)).unwrap(),
            &relative,
        ),
        (
            PlayStart::new(Some(2), Some(Fallback::Unknown)).unwrap(),
            &format!("{held}. {relative}"),
        ),
    ]);
    assert!(relative.starts_with("Playing with relative motion: the screen layout is not known"));
}

#[test]
fn recording_changes_read_as_before() {
    assert_english(&[
        (
            Problem::CurrentUnread(q()),
            "The recording the playback hotkey plays could not be read, so it plays the newest: \
             why",
        ),
        (
            Problem::CurrentUnkept(q()),
            "The recording the playback hotkey plays could not be kept for the next start: why",
        ),
    ]);
    assert_english(&[
        (ChangeFailure::Missing, "the file is no longer there"),
        (ChangeFailure::Refused(q()), "why"),
        (
            ChangeFailure::Unreadable(Unplayable::TooLarge { mib: 256 }),
            "the file is larger than 256 MiB",
        ),
        (ChangeFailure::Folder(q()), "why"),
        (
            ChangeFailure::NotDone {
                step: Step::Written,
                why: q(),
            },
            "the file could not be written: why",
        ),
        (
            ChangeFailure::NotDone {
                step: Step::Deleted,
                why: q(),
            },
            "the file could not be deleted: why",
        ),
        (
            ChangeFailure::TooLarge { mib: 256 },
            "the file could not be written: the renamed recording would be larger than 256 MiB",
        ),
    ]);
}
