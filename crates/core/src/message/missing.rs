//! Why a part the desktop gives is missing: the screen layout, the
//! pointer's position in recordings (W01), or the on-screen display (O01). The backends give these values
//! through the platform; the tray and the settings window only word them.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Why the screen layout is not known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoScreens {
    /// The program was started without `WAYLAND_DISPLAY`.
    NoDisplay,
    /// The display did not answer, or not in time, or another process than
    /// the one found at start listens on it now.
    CannotConnect,
    /// The compositor offers no xdg-output.
    NoXdgOutput,
    /// The connection closed or failed.
    Lost,
    /// The compositor or the program refused what the other sent.
    ProtocolError,
    /// The connection runs, but the compositor has not described every
    /// screen in full (an output too old to bind, none at all, too many).
    NotWhole,
}

impl fmt::Display for NoScreens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NoScreens::NoDisplay => {
                "the program started without a Wayland display (WAYLAND_DISPLAY); see the README"
            }
            NoScreens::CannotConnect => "the program could not connect to the Wayland display",
            NoScreens::NoXdgOutput => {
                "the desktop does not say where its screens are (no xdg-output)"
            }
            NoScreens::Lost => "the connection to the Wayland display ended",
            NoScreens::ProtocolError => "the Wayland display refused the program's requests",
            NoScreens::NotWhole => "the desktop has not described every screen",
        })
    }
}

/// Why nothing reports where the pointer is while recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoPointer {
    /// No compositor of this display reports the pointer: none does on
    /// this desktop, or KWin is not counted (not running, another
    /// display's, or its check not done yet).
    NoSource,
    /// KWin's script cannot be used: the program could not receive from
    /// it, has no folder for it, or its last load failed. The log says why.
    Unusable,
    /// Hyprland's socket did not answer an ask: it could not be reached, or
    /// not before the ask's deadline (W01 brief, "Settled by W01d").
    HyprlandNoAnswer,
    /// Hyprland answered in a form the program does not parse, as a newer
    /// Hyprland might.
    HyprlandUnreadable,
    /// The socket `HYPRLAND_INSTANCE_SIGNATURE` names is not this display's
    /// compositor's, or the signature names no socket of one.
    HyprlandOtherDisplay,
    /// The display's cursor sessions gave no position: their connection
    /// could not be made in time, has no pointer, or ended or lost the
    /// pointer during the recording. The log says which (W02).
    DisplayNoPosition,
}

impl fmt::Display for NoPointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NoPointer::NoSource => "nothing on this desktop reports where the pointer is",
            NoPointer::Unusable => "KWin's pointer script cannot be used; see the log",
            NoPointer::HyprlandNoAnswer => "Hyprland's socket did not answer",
            NoPointer::HyprlandUnreadable => {
                "Hyprland answered in a form this program does not read"
            }
            NoPointer::HyprlandOtherDisplay => "Hyprland's socket belongs to another display",
            NoPointer::DisplayNoPosition => {
                "the Wayland display gave no pointer position; see the log"
            }
        })
    }
}

/// Why the on-screen display cannot show on this desktop (O01's O3): read
/// from what the layout's connection found when it last connected. A
/// failure on one screen, or of the display's own connection, is not one:
/// the log says why, and the next countdown tries again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoOnScreen {
    /// The program was started without `WAYLAND_DISPLAY`.
    NoDisplay,
    /// The display did not answer, or not in time, or refused the program.
    CannotConnect,
    /// The compositor offers no xdg-output, so no screen's size is known.
    NoXdgOutput,
    /// The compositor offers no layer-shell, or not the shared memory and
    /// surfaces it needs.
    NoLayerShell,
}

impl fmt::Display for NoOnScreen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NoOnScreen::NoDisplay => {
                "the program started without a Wayland display (WAYLAND_DISPLAY); see the README"
            }
            NoOnScreen::CannotConnect => "the program could not connect to the Wayland display",
            NoOnScreen::NoXdgOutput => {
                "the desktop does not tell the program its screens' sizes (no xdg-output)"
            }
            NoOnScreen::NoLayerShell => "this desktop cannot show it (no layer-shell)",
        })
    }
}

/// Why recordings get no positions, only movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoPositions {
    /// The screens are not known, so no position can be placed on them.
    Screens(NoScreens),
    /// Nothing reports the pointer.
    Pointer(NoPointer),
}

impl fmt::Display for NoPositions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoPositions::Screens(why) => why.fmt(f),
            NoPositions::Pointer(why) => why.fmt(f),
        }
    }
}
