//! The tooltip's lines in Swedish: what a run does, the hotkeys, how a
//! recording plays and the routes, in the order [`View::details`] gives
//! them in English.

use auto_clicker_core::clicker::{ClickerSettings, Mode, Position, Repeat};
use auto_clicker_core::controller::{Hotkey, NoRecording, Route, Routes, View, duration};
use auto_clicker_core::input::{Button, Key};
use auto_clicker_core::playback::{PlaybackSettings, Speed};
use auto_clicker_core::recording::MAX_DURATION;

use auto_clicker_core::locale::Language;

use super::{label, notice, problem};

/// Every line of the tooltip for `view`, in Swedish.
pub(super) fn details(view: &View) -> Vec<String> {
    let mut lines = vec![describe(&view.settings)];
    lines.extend(hotkey_line(&view.hotkey, "startar och stoppar klickandet"));
    lines.extend(match &view.no_recording {
        Some(no_recording) => no_recording_line(no_recording),
        None => hotkey_line(
            &view.record_hotkey,
            "startar inspelningen, och stoppar och sparar den",
        ),
    });
    lines.extend(hotkey_line(
        &view.play_hotkey,
        &format!(
            "spelar upp den aktuella inspelningen {}, och stoppar den",
            describe_playback(&view.playback)
        ),
    ));
    lines.push(routes_line(view.routes));
    if view.activity.recording() {
        let minutes = MAX_DURATION.as_secs() / 60;
        lines.push(format!(
            "En inspelning stoppas av sig själv efter {minutes} minuter"
        ));
    }
    if let Some(recording) = &view.playing {
        lines.push(format!("Spelar upp {}", recording_name(recording)));
    }
    lines.extend(view.notice.as_ref().map(notice::swedish));
    lines.extend(view.problems.iter().map(problem::swedish));
    lines
}

/// A recording as the menu names it in Swedish.
pub(super) fn recording_name(english: &str) -> String {
    label(english, Language::Swedish)
}

/// "`<hotkey>` `does`", or what to do about a hotkey the desktop has not
/// set; nothing while the desktop has not reported it.
fn hotkey_line(hotkey: &Hotkey, does: &str) -> Option<String> {
    match hotkey {
        Hotkey::Bound(binding) => Some(format!("{binding} {does}")),
        Hotkey::Desktop(None) => None,
        Hotkey::Desktop(Some(trigger)) if trigger.is_empty() => Some(format!(
            "Ingen snabbtangent {does}: välj en i Systeminställningar, under Genvägar"
        )),
        Hotkey::Desktop(Some(trigger)) => Some(format!("{trigger} {does}")),
    }
}

fn no_recording_line(no_recording: &NoRecording) -> Option<String> {
    match no_recording {
        NoRecording::DesktopHotkeys => Some(
            "Inspelning går inte med skrivbordets genvägar: inget läser tangentborden och \
             mössen. Den kräver gruppen input, och hotkeys = \"auto\" eller \"kernel\" under \
             [input] i konfigurationen."
                .to_owned(),
        ),
        NoRecording::Keyboards(_) => None,
    }
}

fn routes_line(routes: Routes) -> String {
    let output = match routes.output {
        Route::Kernel => "Klickar via /dev/uinput",
        Route::Portal => "Klickar via skrivbordets fjärrinmatning",
    };
    let hotkeys = match routes.hotkeys {
        Route::Kernel => "snabbtangenter från tangentborden",
        Route::Portal => "snabbtangenter från skrivbordets genvägar",
    };
    format!("{output}, {hotkeys}")
}

/// One sentence on what a run with `settings` does.
fn describe(settings: &ClickerSettings) -> String {
    let button = button_name(settings.button);
    let key = key_name(settings.key);
    let mut text = match settings.mode {
        Mode::Click => format!("Klickar med {button}"),
        Mode::DoubleClick => format!("Dubbelklickar med {button}"),
        Mode::HoldButton => format!("Håller {button} nedtryckt"),
        Mode::KeyRepeat => format!("Trycker på {key}"),
        Mode::HoldKey => format!("Håller {key} nedtryckt"),
    };
    if settings.mode.repeats() {
        text.push_str(&format!(" var {}", duration(settings.interval.get())));
        if settings.jitter.percent() > 0 {
            text.push_str(&format!(" (±{} %)", settings.jitter.percent()));
        }
        if let Repeat::Times(count) = settings.repeat {
            text.push_str(&format!(", {count} gånger"));
        }
    }
    let uses_button = !matches!(settings.mode, Mode::KeyRepeat | Mode::HoldKey);
    if uses_button && matches!(settings.position, Position::Fixed(_)) {
        text.push_str(" på en fast punkt");
    }
    text
}

/// "en gång", "3 gånger i 1,5× hastighet".
fn describe_playback(settings: &PlaybackSettings) -> String {
    let mut text = match settings.repeat {
        Repeat::UntilStopped => "tills den stoppas".to_owned(),
        Repeat::Times(count) if count.get() == 1 => "en gång".to_owned(),
        Repeat::Times(count) => format!("{count} gånger"),
    };
    if settings.speed != Speed::RECORDED {
        text.push_str(&format!(" i {}× hastighet", decimal(&settings.speed)));
    }
    text
}

/// A number with a decimal comma.
pub(super) fn decimal(number: &impl ToString) -> String {
    number.to_string().replace('.', ",")
}

fn button_name(button: Button) -> &'static str {
    match button {
        Button::Left => "vänsterknappen",
        Button::Right => "högerknappen",
        Button::Middle => "mittenknappen",
        Button::Side => "bakåtknappen",
        Button::Extra => "framåtknappen",
    }
}

/// The key's name, or its code.
pub(super) fn key_name(key: Key) -> String {
    key.name()
        .map_or_else(|| format!("tangent {}", key.code()), str::to_owned)
}
