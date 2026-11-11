//! What the tray shows: the controller's state as plain text.
//!
//! The controller publishes a [`View`] after every change; the tray only
//! reads it. The text is plain: a tray that renders markup escapes it.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::clicker::{ClickerSettings, Mode, Position, Repeat};
use crate::hotkeys::{Binding, DESKTOP_RELEASE_PAUSE, Unavailable};
use crate::input::{Button, Key};
use crate::playback::{PlaybackSettings, Speed};
use crate::recording::{FileName, MAX_DURATION};

/// What the controller is doing, as the tray shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Idle,
    /// A start from the tray counts down; whole seconds left, rounded up.
    StartingIn(u64),
    /// A start from the hotkey waits for its modifiers to come up.
    WaitingForRelease,
    /// A start waits for the input devices to open, which can mean the
    /// desktop's permission dialog.
    Opening,
    Clicking,
    /// A recording from the tray counts down; whole seconds left, rounded
    /// up.
    RecordingIn(u64),
    Recording,
    /// Playback from the tray counts down; whole seconds left, rounded up.
    PlayingIn(u64),
    /// Playback from the hotkey waits for its modifiers to come up.
    PlayWaitingForRelease,
    /// Playback waits for the input devices to open.
    PlayOpening,
    Playing,
}

impl Activity {
    /// Clicking, or about to.
    pub fn clicking(self) -> bool {
        matches!(
            self,
            Activity::StartingIn(_)
                | Activity::WaitingForRelease
                | Activity::Opening
                | Activity::Clicking
        )
    }

    /// Recording, or about to.
    pub fn recording(self) -> bool {
        matches!(self, Activity::RecordingIn(_) | Activity::Recording)
    }

    /// Playing a recording, or about to.
    pub fn playing(self) -> bool {
        matches!(
            self,
            Activity::PlayingIn(_)
                | Activity::PlayWaitingForRelease
                | Activity::PlayOpening
                | Activity::Playing
        )
    }
}

/// One recording in the tray's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// Its file, which [`Message::Play`](super::Message::Play) takes.
    pub file: FileName,
    /// What the tray calls it.
    pub label: String,
}

/// A hotkey as the tray names it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Hotkey {
    /// The config's binding, matched by the program itself.
    Bound(Binding),
    /// The trigger the desktop reports for a hotkey it delivers: `None`
    /// until it reported one, empty when the action has none.
    Desktop(Option<String>),
}

impl Hotkey {
    /// "`<hotkey>` `does`", or what to do about a hotkey the desktop has
    /// not set; nothing while the desktop has not reported it.
    fn line(&self, does: &str) -> Option<String> {
        match self {
            Hotkey::Bound(binding) => Some(format!("{binding} {does}")),
            Hotkey::Desktop(None) => None,
            Hotkey::Desktop(Some(trigger)) if trigger.is_empty() => Some(format!(
                "No hotkey {does}: set one in System Settings, under Shortcuts"
            )),
            Hotkey::Desktop(Some(trigger)) => Some(format!("{trigger} {does}")),
        }
    }

    /// What a start from this hotkey waits for.
    fn start_wait(&self) -> String {
        match self {
            Hotkey::Bound(binding) => format!("when you let go of {}", modifiers_of(*binding)),
            Hotkey::Desktop(_) => format!(
                "{} s after you let go of the hotkey",
                DESKTOP_RELEASE_PAUSE.as_secs()
            ),
        }
    }
}

/// Why no recording can start here, whatever state the controller is in.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoRecording {
    /// The desktop delivers the hotkeys (`hotkeys = "portal"`): nothing
    /// reads the keyboards and mice a recording is made of.
    DesktopHotkeys,
    /// The user may not read the keyboards, or watching for them failed. The
    /// tooltip lists this as a problem already, with what to do.
    Keyboards(Unavailable),
}

/// The refusal on the desktop's hotkeys, after "Recording is".
const WITH_DESKTOP_HOTKEYS: &str = "unavailable with the desktop's shortcuts";

impl NoRecording {
    /// What the tray's Record item reads: the item and why it is off. The
    /// desktop's shortcuts are in use either because the config asks for
    /// them or because the keyboards could not be read at start; the input
    /// group is what recording needs in both cases.
    pub fn label(&self) -> &'static str {
        match self {
            NoRecording::DesktopHotkeys | NoRecording::Keyboards(Unavailable::NoPermission) => {
                "Record (needs the input group)"
            }
            NoRecording::Keyboards(_) => "Record (needs a restart)",
        }
    }

    /// The tooltip's line on it, in place of the recording hotkey's: none
    /// where a problem line says why already.
    fn line(&self) -> Option<String> {
        match self {
            NoRecording::DesktopHotkeys => Some(format!(
                "Recording is {WITH_DESKTOP_HOTKEYS}: nothing reads the keyboards and mice. It \
                 needs the input group, and hotkeys = \"auto\" or \"kernel\" under [input] in the \
                 config."
            )),
            NoRecording::Keyboards(_) => None,
        }
    }
}

/// Why a recording did not start, after "Not recording: ". The tooltip's
/// own line or problem says what to do.
impl fmt::Display for NoRecording {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoRecording::DesktopHotkeys => write!(f, "recording is {WITH_DESKTOP_HOTKEYS}"),
            NoRecording::Keyboards(why) => why.fmt(f),
        }
    }
}

/// A route a job takes, as the daemon picked it at start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// The kernel: `/dev/uinput` for clicks, `/dev/input` for hotkeys.
    #[default]
    Kernel,
    /// The desktop's portals: remote input for clicks, its shortcuts for
    /// hotkeys.
    Portal,
}

/// The routes the daemon runs on. They are picked once at start, so they
/// change only with a restart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Routes {
    /// Where clicks and keys go.
    pub output: Route,
    /// Where hotkeys come from.
    pub hotkeys: Route,
}

impl Routes {
    /// The tooltip's line naming both.
    pub fn line(self) -> String {
        let output = match self.output {
            Route::Kernel => "Clicks through /dev/uinput",
            Route::Portal => "Clicks through the desktop's remote input",
        };
        let hotkeys = match self.hotkeys {
            Route::Kernel => "hotkeys from the keyboards",
            Route::Portal => "hotkeys from the desktop's shortcuts",
        };
        format!("{output}, {hotkeys}")
    }
}

/// Everything the tray shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub activity: Activity,
    /// The routes in use.
    pub routes: Routes,
    pub settings: ClickerSettings,
    /// The hotkey that starts and stops clicking.
    pub hotkey: Hotkey,
    /// The hotkey that starts and stops recording.
    pub record_hotkey: Hotkey,
    /// Why no recording can start, if none can: the tray's Record item is
    /// off then.
    pub no_recording: Option<NoRecording>,
    /// The hotkey that starts and stops playback.
    pub play_hotkey: Hotkey,
    pub playback: PlaybackSettings,
    /// The newest recordings, newest first: what the tray offers to play.
    pub recordings: Arc<[Listed]>,
    /// The recording that plays or is about to, as the tray names it.
    pub playing: Option<String>,
    /// What last happened that the user should know: why a start was
    /// refused or cancelled, or why a run stopped.
    pub notice: Option<String>,
    /// What is wrong now: an invalid config, unreadable keyboards, a locked
    /// session, devices that could not be opened.
    pub problems: Vec<String>,
}

impl View {
    /// One line on the state, for the tray's title and tooltip title.
    pub fn title(&self) -> String {
        match self.activity {
            Activity::Idle => "Auto clicker: idle".to_owned(),
            Activity::StartingIn(seconds) => format!("Auto clicker: starts in {seconds} s"),
            Activity::WaitingForRelease => {
                format!("Auto clicker: starts {}", self.hotkey.start_wait())
            }
            Activity::Opening => {
                "Auto clicker: starting; allow input if the desktop asks".to_owned()
            }
            Activity::Clicking => "Auto clicker: clicking".to_owned(),
            Activity::RecordingIn(seconds) => format!("Auto clicker: records in {seconds} s"),
            Activity::Recording => "Auto clicker: recording".to_owned(),
            Activity::PlayingIn(seconds) => format!("Auto clicker: plays in {seconds} s"),
            Activity::PlayWaitingForRelease => {
                format!("Auto clicker: plays {}", self.play_hotkey.start_wait())
            }
            Activity::PlayOpening => {
                "Auto clicker: starting playback; allow input if the desktop asks".to_owned()
            }
            Activity::Playing => "Auto clicker: playing".to_owned(),
        }
    }

    /// The tooltip's lines: what a run does, the hotkeys, how a recording
    /// plays, the routes in use, the recording's time limit while one runs,
    /// what plays, the last notice and every problem.
    pub fn details(&self) -> Vec<String> {
        let mut lines = vec![describe(&self.settings)];
        lines.extend(self.hotkey.line("starts and stops clicking"));
        lines.extend(match &self.no_recording {
            Some(no_recording) => no_recording.line(),
            None => self
                .record_hotkey
                .line("starts recording, and stops and saves it"),
        });
        lines.extend(self.play_hotkey.line(&format!(
            "plays the last recording {}, and stops it",
            describe_playback(&self.playback)
        )));
        lines.push(self.routes.line());
        if self.activity.recording() {
            let minutes = MAX_DURATION.as_secs() / 60;
            lines.push(format!(
                "A recording stops by itself after {minutes} minutes"
            ));
        }
        if let Some(recording) = &self.playing {
            lines.push(format!("Playing {recording}"));
        }
        lines.extend(self.notice.iter().cloned());
        lines.extend(self.problems.iter().cloned());
        lines
    }
}

/// The binding's modifiers, "Ctrl" or "Ctrl and Shift"; "the hotkey" when it
/// has none.
fn modifiers_of(binding: Binding) -> String {
    let names: Vec<&str> = binding.modifiers().iter().map(|m| m.name()).collect();
    match names.split_last() {
        None => "the hotkey".to_owned(),
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// One sentence on what a run with `settings` does.
pub fn describe(settings: &ClickerSettings) -> String {
    let button = button_name(settings.button);
    let key = key_name(settings.key);
    let action = match settings.mode {
        Mode::Click => format!("Clicks the {button} button"),
        Mode::DoubleClick => format!("Double-clicks the {button} button"),
        Mode::HoldButton => format!("Holds the {button} button down"),
        Mode::KeyRepeat => format!("Presses {key}"),
        Mode::HoldKey => format!("Holds {key} down"),
    };
    let mut text = action;
    if settings.mode.repeats() {
        text.push_str(&format!(" every {}", duration(settings.interval.get())));
        if settings.jitter.percent() > 0 {
            text.push_str(&format!(" (±{}%)", settings.jitter.percent()));
        }
        if let Repeat::Times(count) = settings.repeat {
            text.push_str(&format!(", {count} times"));
        }
    }
    let uses_button = !matches!(settings.mode, Mode::KeyRepeat | Mode::HoldKey);
    if uses_button && matches!(settings.position, Position::Fixed(_)) {
        text.push_str(" at a fixed point");
    }
    text
}

/// How often and how fast a recording plays: "once", "3 times at 1.5× speed".
pub fn describe_playback(settings: &PlaybackSettings) -> String {
    let mut text = match settings.repeat {
        Repeat::UntilStopped => "until stopped".to_owned(),
        Repeat::Times(count) if count.get() == 1 => "once".to_owned(),
        Repeat::Times(count) => format!("{count} times"),
    };
    if settings.speed != Speed::RECORDED {
        text.push_str(&format!(" at {}× speed", settings.speed));
    }
    text
}

/// The button as the config file names it.
fn button_name(button: Button) -> &'static str {
    match button {
        Button::Left => "left",
        Button::Right => "right",
        Button::Middle => "middle",
        Button::Side => "back",
        Button::Extra => "forward",
    }
}

fn key_name(key: Key) -> String {
    key.name()
        .map_or_else(|| format!("key {}", key.code()), str::to_owned)
}

/// "100 ms", or "2 s" for whole seconds.
fn duration(interval: Duration) -> String {
    let millis = interval.as_millis();
    if millis >= 1000 && millis.is_multiple_of(1000) {
        format!("{} s", millis / 1000)
    } else {
        format!("{millis} ms")
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::*;
    use crate::clicker::{Interval, Jitter};
    use crate::input::AbsolutePosition;

    fn settings(mode: Mode) -> ClickerSettings {
        ClickerSettings {
            mode,
            ..ClickerSettings::default()
        }
    }

    #[test]
    fn describes_each_mode() {
        let cases = [
            (Mode::Click, "Clicks the left button every 100 ms"),
            (
                Mode::DoubleClick,
                "Double-clicks the left button every 100 ms",
            ),
            (Mode::HoldButton, "Holds the left button down"),
            (Mode::KeyRepeat, "Presses space every 100 ms"),
            (Mode::HoldKey, "Holds space down"),
        ];
        for (mode, text) in cases {
            assert_eq!(describe(&settings(mode)), text, "{mode:?}");
        }
    }

    #[test]
    fn describes_jitter_count_position_and_whole_seconds() {
        let settings = ClickerSettings {
            interval: Interval::new(Duration::from_secs(2)).unwrap(),
            jitter: Jitter::new(10).unwrap(),
            button: Button::Side,
            repeat: Repeat::Times(NonZeroU32::new(50).unwrap()),
            position: Position::Fixed(AbsolutePosition { x: 1, y: 2 }),
            ..ClickerSettings::default()
        };
        assert_eq!(
            describe(&settings),
            "Clicks the back button every 2 s (±10%), 50 times at a fixed point"
        );
    }

    #[test]
    fn only_whole_seconds_are_shown_in_seconds() {
        let shown = |ms| duration(Duration::from_millis(ms));
        assert_eq!(shown(1500), "1500 ms");
        assert_eq!(shown(1000), "1 s");
        assert_eq!(shown(100), "100 ms");
    }

    #[test]
    fn a_key_mode_ignores_the_fixed_point_and_names_unnamed_keys_by_code() {
        let settings = ClickerSettings {
            mode: Mode::HoldKey,
            key: Key::new(248).unwrap(),
            position: Position::Fixed(AbsolutePosition { x: 1, y: 2 }),
            ..ClickerSettings::default()
        };
        assert_eq!(describe(&settings), "Holds key 248 down");
    }

    fn view(activity: Activity, hotkey: &str) -> View {
        View {
            activity,
            routes: Routes::default(),
            settings: ClickerSettings::default(),
            hotkey: Hotkey::Bound(Binding::parse(hotkey).unwrap()),
            record_hotkey: Hotkey::Bound(Binding::parse("Ctrl+F7").unwrap()),
            no_recording: None,
            play_hotkey: Hotkey::Bound(Binding::parse("Ctrl+Alt+F8").unwrap()),
            playback: PlaybackSettings::default(),
            recordings: Arc::from([]),
            playing: None,
            notice: None,
            problems: Vec::new(),
        }
    }

    #[test]
    fn titles_name_the_state() {
        let cases = [
            (Activity::Idle, "Ctrl+F6", "Auto clicker: idle"),
            (
                Activity::StartingIn(3),
                "Ctrl+F6",
                "Auto clicker: starts in 3 s",
            ),
            (
                Activity::WaitingForRelease,
                "Ctrl+F6",
                "Auto clicker: starts when you let go of Ctrl",
            ),
            (
                Activity::WaitingForRelease,
                "Ctrl+Shift+Alt+F6",
                "Auto clicker: starts when you let go of Ctrl, Shift and Alt",
            ),
            (
                Activity::WaitingForRelease,
                "F13",
                "Auto clicker: starts when you let go of the hotkey",
            ),
            (Activity::Clicking, "Ctrl+F6", "Auto clicker: clicking"),
            (
                Activity::RecordingIn(2),
                "Ctrl+F6",
                "Auto clicker: records in 2 s",
            ),
            (Activity::Recording, "Ctrl+F6", "Auto clicker: recording"),
            (
                Activity::PlayingIn(4),
                "Ctrl+F6",
                "Auto clicker: plays in 4 s",
            ),
            (
                Activity::PlayWaitingForRelease,
                "Ctrl+F6",
                "Auto clicker: plays when you let go of Ctrl and Alt",
            ),
            (
                Activity::PlayOpening,
                "Ctrl+F6",
                "Auto clicker: starting playback; allow input if the desktop asks",
            ),
            (Activity::Playing, "Ctrl+F6", "Auto clicker: playing"),
        ];
        for (activity, hotkey, title) in cases {
            assert_eq!(view(activity, hotkey).title(), title);
        }
    }

    #[test]
    fn details_list_settings_hotkey_notice_and_problems() {
        let mut shown = view(Activity::Idle, "Ctrl+F6");
        shown.hotkey = Hotkey::Bound(Binding::parse("Ctrl+F6").unwrap());
        shown.notice = Some("notice".to_owned());
        shown.problems = vec!["one".to_owned(), "two".to_owned()];
        assert_eq!(
            shown.details(),
            [
                "Clicks the left button every 100 ms",
                "Ctrl+F6 starts and stops clicking",
                "Ctrl+F7 starts recording, and stops and saves it",
                "Ctrl+Alt+F8 plays the last recording once, and stops it",
                "Clicks through /dev/uinput, hotkeys from the keyboards",
                "notice",
                "one",
                "two",
            ]
        );
    }

    #[test]
    fn a_recording_that_cannot_start_replaces_the_recording_hotkeys_line() {
        let portal = "Recording is unavailable with the desktop's shortcuts: nothing reads the \
                      keyboards and mice. It needs the input group, and hotkeys = \"auto\" or \
                      \"kernel\" under [input] in the config.";
        let keyboards = NoRecording::Keyboards(Unavailable::NoPermission);
        let watch = NoRecording::Keyboards(Unavailable::WatchFailed("gone".to_owned()));
        // Every state of the hotkey, as the program or the desktop names it:
        // none of them is offered for recording.
        let hotkeys = [
            Hotkey::Bound(Binding::parse("Ctrl+F7").unwrap()),
            Hotkey::Desktop(None),
            Hotkey::Desktop(Some(String::new())),
            Hotkey::Desktop(Some("Ctrl+Shift+F7".to_owned())),
        ];
        for record_hotkey in hotkeys {
            for (no_recording, line) in [
                (NoRecording::DesktopHotkeys, Some(portal)),
                // The problem line says why already.
                (keyboards.clone(), None),
                (watch.clone(), None),
            ] {
                let shown = View {
                    record_hotkey: record_hotkey.clone(),
                    no_recording: Some(no_recording.clone()),
                    ..view(Activity::Idle, "Ctrl+F6")
                };
                let expected: Vec<&str> = [
                    Some("Clicks the left button every 100 ms"),
                    Some("Ctrl+F6 starts and stops clicking"),
                    line,
                    Some("Ctrl+Alt+F8 plays the last recording once, and stops it"),
                    Some("Clicks through /dev/uinput, hotkeys from the keyboards"),
                ]
                .into_iter()
                .flatten()
                .collect();
                assert_eq!(
                    shown.details(),
                    expected,
                    "{record_hotkey:?} {no_recording:?}"
                );
            }
        }
    }

    #[test]
    fn each_reason_has_its_label_and_refusal() {
        let cases = [
            (
                NoRecording::DesktopHotkeys,
                "Record (needs the input group)",
                "recording is unavailable with the desktop's shortcuts",
            ),
            (
                NoRecording::Keyboards(Unavailable::NoPermission),
                "Record (needs the input group)",
                "hotkeys are unavailable: keyboards cannot be read; add your user to the input \
                 group and log in again, or set hotkeys = \"portal\" under [input] for the \
                 desktop's shortcuts",
            ),
            (
                NoRecording::Keyboards(Unavailable::WatchFailed("gone".to_owned())),
                "Record (needs a restart)",
                "hotkeys stopped: watching for keyboards failed: gone; restart the program",
            ),
        ];
        for (no_recording, label, refusal) in cases {
            assert_eq!(no_recording.label(), label);
            assert_eq!(no_recording.to_string(), refusal);
        }
    }

    #[test]
    fn a_recording_shows_its_time_limit() {
        for (activity, shown) in [
            (Activity::Recording, true),
            (Activity::RecordingIn(3), true),
            (Activity::Clicking, false),
            (Activity::Idle, false),
        ] {
            let lines = view(activity, "Ctrl+F6").details();
            let limit = "A recording stops by itself after 60 minutes".to_owned();
            assert_eq!(lines.contains(&limit), shown, "{activity:?}");
        }
    }

    #[test]
    fn what_plays_is_named_before_the_notice() {
        let mut shown = view(Activity::Playing, "Ctrl+F6");
        shown.playing = Some("Recording 2026-11-09 18:04:05".to_owned());
        shown.notice = Some("notice".to_owned());
        let lines = shown.details();
        assert_eq!(
            lines[lines.len() - 2..],
            ["Playing Recording 2026-11-09 18:04:05", "notice"]
        );
    }

    #[test]
    fn describes_how_a_recording_plays() {
        let described = |speed, repeat| {
            describe_playback(&PlaybackSettings {
                speed: Speed::from_ratio(speed).unwrap(),
                repeat,
            })
        };
        let times = |count| Repeat::Times(NonZeroU32::new(count).unwrap());
        assert_eq!(described(1.0, times(1)), "once");
        assert_eq!(described(1.0, times(3)), "3 times");
        assert_eq!(described(1.0, Repeat::UntilStopped), "until stopped");
        assert_eq!(described(1.5, times(1)), "once at 1.5× speed");
        assert_eq!(
            described(0.25, Repeat::UntilStopped),
            "until stopped at 0.25× speed"
        );
    }

    #[test]
    fn activities_are_clicking_or_recording_or_playing_or_none() {
        use Activity::*;
        let kinds = [
            Idle,
            StartingIn(1),
            WaitingForRelease,
            Opening,
            Clicking,
            RecordingIn(1),
            Recording,
            PlayingIn(1),
            PlayWaitingForRelease,
            PlayOpening,
            Playing,
        ]
        .map(|activity| {
            (
                activity.clicking(),
                activity.recording(),
                activity.playing(),
            )
        });
        let no = (false, false, false);
        let (click, record) = ((true, false, false), (false, true, false));
        let play = (false, false, true);
        assert_eq!(
            kinds,
            [
                no, click, click, click, click, record, record, play, play, play, play
            ]
        );
    }
}
