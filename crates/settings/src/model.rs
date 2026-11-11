//! The window's fields and the draft they stand for, both ways: the fields
//! show the settings the daemon sent, and give back a draft for a save. The
//! daemon checks what it gets; the window only reads numbers out of text.

use auto_clicker_core::config::{
    ButtonName, ClickerDraft, CountDraft, Draft, HotkeysDraft, InputDraft, IntervalDraft, ModeName,
    PlaybackDraft, PositionDraft, RouteName,
};
use auto_clicker_core::controller::Hotkey;
use auto_clicker_core::input::Key;

/// The modes in the order the window lists them.
const MODES: [ModeName; 5] = [
    ModeName::Click,
    ModeName::DoubleClick,
    ModeName::HoldButton,
    ModeName::KeyRepeat,
    ModeName::HoldKey,
];

/// The buttons in the order the window lists them.
const BUTTONS: [ButtonName; 5] = [
    ButtonName::Left,
    ButtonName::Right,
    ButtonName::Middle,
    ButtonName::Back,
    ButtonName::Forward,
];

/// The routes in the order the window lists them.
const ROUTES: [RouteName; 2] = [RouteName::Kernel, RouteName::Portal];

/// What the window's fields hold, as the window's widgets take it.
#[derive(Clone, Debug, PartialEq)]
pub struct Fields {
    pub tray_delay: i32,
    pub mode: i32,
    pub interval_text: String,
    /// 0: milliseconds between actions; 1: actions a second.
    pub interval_unit: i32,
    pub jitter: i32,
    pub button: i32,
    /// The clicker's key: a key name, or a Linux key code.
    pub key_text: String,
    pub position_fixed: bool,
    pub position_x: i32,
    pub position_y: i32,
    pub repeat_forever: bool,
    pub repeat_text: String,
    pub output_route: i32,
    pub hotkeys_route: i32,
    pub speed_text: String,
    pub plays_forever: bool,
    pub plays_text: String,
}

/// A field whose text cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unreadable {
    Interval,
    Key,
    Repeat,
    Speed,
    Plays,
}

impl Fields {
    /// The fields that show `draft`, writing decimals with `decimal` as the
    /// separator, as the window's language does.
    pub fn from_draft(draft: &Draft, decimal: char) -> Fields {
        let clicker = &draft.clicker;
        let (interval_text, interval_unit) = match clicker.interval {
            IntervalDraft::Ms(ms) => (ms.to_string(), 0),
            IntervalDraft::PerSecond(rate) => (decimal_text(rate, decimal), 1),
        };
        let (position_fixed, position_x, position_y) = match clicker.position {
            PositionDraft::Cursor => (false, 0, 0),
            PositionDraft::Fixed { x, y } => (true, i32::from(x), i32::from(y)),
        };
        let (repeat_forever, repeat_text) = count_fields(clicker.repeat);
        let (plays_forever, plays_text) = count_fields(draft.playback.repeat);
        Fields {
            tray_delay: to_field(draft.tray_start_delay),
            mode: place(&MODES, clicker.mode),
            interval_text,
            interval_unit,
            jitter: to_field(clicker.jitter_percent),
            button: place(&BUTTONS, clicker.button),
            key_text: key_text(clicker.key),
            position_fixed,
            position_x,
            position_y,
            repeat_forever,
            repeat_text,
            output_route: place(&ROUTES, draft.input.output),
            hotkeys_route: place(&ROUTES, draft.input.hotkeys),
            speed_text: decimal_text(draft.playback.speed, decimal),
            plays_forever,
            plays_text,
        }
    }

    /// The draft these fields give, with the hotkeys of `hotkeys` (which
    /// only a capture changes); which field cannot be read, if one cannot.
    /// A decimal is read with a point or a comma.
    pub fn to_draft(&self, hotkeys: &HotkeysDraft) -> Result<Draft, Unreadable> {
        let interval = match self.interval_unit {
            1 => IntervalDraft::PerSecond(decimal(&self.interval_text, Unreadable::Interval)?),
            _ => IntervalDraft::Ms(whole(&self.interval_text, Unreadable::Interval)?),
        };
        let position = if self.position_fixed {
            PositionDraft::Fixed {
                x: coordinate(self.position_x),
                y: coordinate(self.position_y),
            }
        } else {
            PositionDraft::Cursor
        };
        Ok(Draft {
            tray_start_delay: from_field(self.tray_delay),
            clicker: ClickerDraft {
                mode: chosen(&MODES, self.mode, ModeName::Click),
                interval,
                jitter_percent: from_field(self.jitter),
                button: chosen(&BUTTONS, self.button, ButtonName::Left),
                key: key_code(&self.key_text)?,
                position,
                repeat: count_draft(self.repeat_forever, &self.repeat_text, Unreadable::Repeat)?,
            },
            hotkeys: hotkeys.clone(),
            input: InputDraft {
                output: chosen(&ROUTES, self.output_route, RouteName::Kernel),
                hotkeys: chosen(&ROUTES, self.hotkeys_route, RouteName::Kernel),
            },
            playback: PlaybackDraft {
                speed: decimal(&self.speed_text, Unreadable::Speed)?,
                repeat: count_draft(self.plays_forever, &self.plays_text, Unreadable::Plays)?,
            },
        })
    }
}

/// A hotkey as the window shows it; `not_set` and `not_reported` stand for
/// a trigger the desktop reports as none, and one it has not reported.
pub fn hotkey_text(hotkey: &Hotkey, not_set: &str, not_reported: &str) -> String {
    match hotkey {
        Hotkey::Bound(binding) => binding.to_string(),
        Hotkey::Desktop(Some(trigger)) if trigger.is_empty() => not_set.to_owned(),
        Hotkey::Desktop(Some(trigger)) => trigger.clone(),
        Hotkey::Desktop(None) => not_reported.to_owned(),
    }
}

/// The clicker's key as the window shows it: its name, or its code.
pub fn key_text(code: u16) -> String {
    match Key::new(code).and_then(Key::name) {
        Some(name) => name.to_owned(),
        None => code.to_string(),
    }
}

/// The hotkeys of `hotkeys` with `action`'s (0 to 2, in the window's
/// order) replaced by `binding`.
pub fn with_hotkey(hotkeys: &HotkeysDraft, action: i32, binding: String) -> HotkeysDraft {
    let mut changed = hotkeys.clone();
    match action {
        0 => changed.toggle_clicking = binding,
        1 => changed.toggle_recording = binding,
        _ => changed.toggle_playback = binding,
    }
    changed
}

/// A whole number written in `text`.
fn whole(text: &str, field: Unreadable) -> Result<u64, Unreadable> {
    text.trim().parse().map_err(|_| field)
}

/// A decimal written in `text`, with a point or a comma; a number that is
/// not finite is none (it has no JSON form, and no setting takes it).
fn decimal(text: &str, field: Unreadable) -> Result<f64, Unreadable> {
    text.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or(field)
}

/// `value` written with `separator` between its whole and its fraction.
fn decimal_text(value: f64, separator: char) -> String {
    value.to_string().replace('.', &separator.to_string())
}

/// The key `text` names: a key name in any letter case, or a key code.
/// The daemon checks a code is one.
fn key_code(text: &str) -> Result<u16, Unreadable> {
    let text = text.trim();
    Key::from_name(&text.to_ascii_lowercase())
        .map(Key::code)
        .or_else(|| text.parse().ok())
        .ok_or(Unreadable::Key)
}

fn count_fields(count: CountDraft) -> (bool, String) {
    match count {
        CountDraft::UntilStopped => (true, "1".to_owned()),
        CountDraft::Times(times) => (false, times.to_string()),
    }
}

/// The count the fields give: until stopped, or the number written.
fn count_draft(forever: bool, text: &str, field: Unreadable) -> Result<CountDraft, Unreadable> {
    if forever {
        Ok(CountDraft::UntilStopped)
    } else {
        whole(text, field).map(CountDraft::Times)
    }
}

/// A number for a field, which holds `i32`; the daemon refuses one too
/// large for it anyway.
fn to_field(value: u64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// A field's number for the draft; a negative one, which no widget gives,
/// becomes 0, which the daemon refuses where it is no value.
fn from_field(value: i32) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn coordinate(value: i32) -> u16 {
    u16::try_from(value.clamp(0, i32::from(u16::MAX))).unwrap_or(u16::MAX)
}

/// Where `value` is in `list`, for a combo box.
fn place<T: PartialEq>(list: &[T], value: T) -> i32 {
    list.iter()
        .position(|known| *known == value)
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(0)
}

/// What a combo box's `index` chose in `list`; `otherwise` for an index
/// no widget gives.
fn chosen<T: Copy>(list: &[T], index: i32, otherwise: T) -> T {
    usize::try_from(index)
        .ok()
        .and_then(|index| list.get(index))
        .copied()
        .unwrap_or(otherwise)
}

#[cfg(test)]
mod tests;
