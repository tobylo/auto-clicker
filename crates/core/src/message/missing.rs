//! Why a part the desktop gives is missing: the screen layout, or the
//! pointer's position in recordings (W01). The backends give these values
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
}

impl fmt::Display for NoPointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NoPointer::NoSource => "nothing on this desktop reports where the pointer is",
            NoPointer::Unusable => "KWin's pointer script cannot be used; see the log",
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
