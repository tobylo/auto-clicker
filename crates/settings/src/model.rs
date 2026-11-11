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
    pub position_fixed: bool,
    pub position_x: i32,
    pub position_y: i32,
    pub repeat_forever: bool,
    pub repeat_count: i32,
    pub output_route: i32,
    pub hotkeys_route: i32,
    pub speed_text: String,
    pub plays_forever: bool,
    pub plays_count: i32,
}

/// A field whose text is not a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotANumber {
    Interval,
    Speed,
}

impl Fields {
    /// The fields that show `draft`.
    pub fn from_draft(draft: &Draft) -> Fields {
        let clicker = &draft.clicker;
        let (interval_text, interval_unit) = match clicker.interval {
            IntervalDraft::Ms(ms) => (ms.to_string(), 0),
            IntervalDraft::PerSecond(rate) => (rate.to_string(), 1),
        };
        let (position_fixed, position_x, position_y) = match clicker.position {
            PositionDraft::Cursor => (false, 0, 0),
            PositionDraft::Fixed { x, y } => (true, i32::from(x), i32::from(y)),
        };
        let (repeat_forever, repeat_count) = count_fields(clicker.repeat);
        let (plays_forever, plays_count) = count_fields(draft.playback.repeat);
        Fields {
            tray_delay: to_field(draft.tray_start_delay),
            mode: place(&MODES, clicker.mode),
            interval_text,
            interval_unit,
            jitter: to_field(clicker.jitter_percent),
            button: place(&BUTTONS, clicker.button),
            position_fixed,
            position_x,
            position_y,
            repeat_forever,
            repeat_count,
            output_route: place(&ROUTES, draft.input.output),
            hotkeys_route: place(&ROUTES, draft.input.hotkeys),
            speed_text: draft.playback.speed.to_string(),
            plays_forever,
            plays_count,
        }
    }

    /// The draft these fields give, with the hotkeys and the clicker's key
    /// of `keys` (which only a capture changes); which field is no number,
    /// if one is not.
    pub fn to_draft(&self, keys: &Draft) -> Result<Draft, NotANumber> {
        let interval = match self.interval_unit {
            1 => IntervalDraft::PerSecond(number(&self.interval_text, NotANumber::Interval)?),
            _ => IntervalDraft::Ms(number(&self.interval_text, NotANumber::Interval)?),
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
                key: keys.clicker.key,
                position,
                repeat: count_draft(self.repeat_forever, self.repeat_count),
            },
            hotkeys: keys.hotkeys.clone(),
            input: InputDraft {
                output: chosen(&ROUTES, self.output_route, RouteName::Kernel),
                hotkeys: chosen(&ROUTES, self.hotkeys_route, RouteName::Kernel),
            },
            playback: PlaybackDraft {
                speed: number(&self.speed_text, NotANumber::Speed)?,
                repeat: count_draft(self.plays_forever, self.plays_count),
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

fn number<T: std::str::FromStr>(text: &str, field: NotANumber) -> Result<T, NotANumber> {
    text.trim().parse().map_err(|_| field)
}

fn count_fields(count: CountDraft) -> (bool, i32) {
    match count {
        CountDraft::UntilStopped => (true, 1),
        CountDraft::Times(times) => (false, to_field(times)),
    }
}

fn count_draft(forever: bool, count: i32) -> CountDraft {
    if forever {
        CountDraft::UntilStopped
    } else {
        CountDraft::Times(from_field(count))
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
