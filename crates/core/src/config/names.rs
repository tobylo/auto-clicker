//! The words the config file uses for the settings that are one of a few
//! things: modes, buttons and routes. The file and the settings window spell
//! them alike, so both read them through these types.

use serde::{Deserialize, Serialize};

use super::{HotkeyRoute, OutputRoute};
use crate::clicker::Mode;
use crate::input::Button;

/// A route, as `[input]` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteName {
    Kernel,
    Portal,
}

impl RouteName {
    pub fn word(self) -> &'static str {
        match self {
            RouteName::Kernel => "kernel",
            RouteName::Portal => "portal",
        }
    }
}

impl From<RouteName> for OutputRoute {
    fn from(name: RouteName) -> OutputRoute {
        match name {
            RouteName::Kernel => OutputRoute::Kernel,
            RouteName::Portal => OutputRoute::Portal,
        }
    }
}

impl From<RouteName> for HotkeyRoute {
    fn from(name: RouteName) -> HotkeyRoute {
        match name {
            RouteName::Kernel => HotkeyRoute::Kernel,
            RouteName::Portal => HotkeyRoute::Portal,
        }
    }
}

impl From<OutputRoute> for RouteName {
    fn from(route: OutputRoute) -> RouteName {
        match route {
            OutputRoute::Kernel => RouteName::Kernel,
            OutputRoute::Portal => RouteName::Portal,
        }
    }
}

impl From<HotkeyRoute> for RouteName {
    fn from(route: HotkeyRoute) -> RouteName {
        match route {
            HotkeyRoute::Kernel => RouteName::Kernel,
            HotkeyRoute::Portal => RouteName::Portal,
        }
    }
}

/// A clicker mode, as `mode` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeName {
    Click,
    DoubleClick,
    HoldButton,
    KeyRepeat,
    HoldKey,
}

impl ModeName {
    pub fn word(self) -> &'static str {
        match self {
            ModeName::Click => "click",
            ModeName::DoubleClick => "double_click",
            ModeName::HoldButton => "hold_button",
            ModeName::KeyRepeat => "key_repeat",
            ModeName::HoldKey => "hold_key",
        }
    }
}

impl From<ModeName> for Mode {
    fn from(name: ModeName) -> Mode {
        match name {
            ModeName::Click => Mode::Click,
            ModeName::DoubleClick => Mode::DoubleClick,
            ModeName::HoldButton => Mode::HoldButton,
            ModeName::KeyRepeat => Mode::KeyRepeat,
            ModeName::HoldKey => Mode::HoldKey,
        }
    }
}

impl From<Mode> for ModeName {
    fn from(mode: Mode) -> ModeName {
        match mode {
            Mode::Click => ModeName::Click,
            Mode::DoubleClick => ModeName::DoubleClick,
            Mode::HoldButton => ModeName::HoldButton,
            Mode::KeyRepeat => ModeName::KeyRepeat,
            Mode::HoldKey => ModeName::HoldKey,
        }
    }
}

/// A mouse button, as `button` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonName {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl ButtonName {
    pub fn word(self) -> &'static str {
        match self {
            ButtonName::Left => "left",
            ButtonName::Right => "right",
            ButtonName::Middle => "middle",
            ButtonName::Back => "back",
            ButtonName::Forward => "forward",
        }
    }
}

impl From<ButtonName> for Button {
    fn from(name: ButtonName) -> Button {
        match name {
            ButtonName::Left => Button::Left,
            ButtonName::Right => Button::Right,
            ButtonName::Middle => Button::Middle,
            ButtonName::Back => Button::Side,
            ButtonName::Forward => Button::Extra,
        }
    }
}

impl From<Button> for ButtonName {
    fn from(button: Button) -> ButtonName {
        match button {
            Button::Left => ButtonName::Left,
            Button::Right => ButtonName::Right,
            Button::Middle => ButtonName::Middle,
            Button::Side => ButtonName::Back,
            Button::Extra => ButtonName::Forward,
        }
    }
}
