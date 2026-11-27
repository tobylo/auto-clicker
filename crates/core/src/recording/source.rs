//! What reported a recording's positions, and how that decides the way
//! they play (W01 brief, "Settled by W01d"), and on which system they can
//! play at all (M5 brief decision 11).

use super::{Layout, Point};
use crate::input::AbsolutePosition;

/// What reported a recording's positions, which decides how they play.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerSource {
    /// KWin's position script (version 2): positions rounded to the nearest
    /// whole logical pixel, and none while a game holds the pointer.
    #[default]
    Kwin,
    /// Hyprland's `cursorpos` (version 3): positions floored to whole
    /// logical pixels, and a game's locked pointer drifts, so its positions
    /// can cover the game's moves (decision 0004).
    Hyprland,
    /// A Wayland display's cursor sessions (ext-image-copy-capture, niri):
    /// physical pixels mapped to the nearest whole logical pixel, and none
    /// while a game holds the pointer, which does not move then (W02). They
    /// play as KWin's and are written as version 2, which reads back as
    /// [`PointerSource::Kwin`].
    CursorSessions,
    /// Windows' pointer (version 3): whole physical pixels, on monitors in
    /// physical pixels (M5 brief decision 10).
    Windows,
}

/// The system a recording's positions belong to: they mean the same
/// place only on the system that took them, whatever its screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    /// Linux: logical pixels, from KWin, Hyprland or a display's cursor
    /// sessions.
    Linux,
    /// Windows: physical pixels.
    Windows,
}

impl System {
    /// The system this program runs on.
    pub const CURRENT: System = if cfg!(windows) {
        System::Windows
    } else {
        System::Linux
    };
}

impl PointerSource {
    /// The system whose screens the positions lie on.
    pub fn system(self) -> System {
        match self {
            PointerSource::Kwin | PointerSource::Hyprland | PointerSource::CursorSessions => {
                System::Linux
            }
            PointerSource::Windows => System::Windows,
        }
    }

    /// Whether every relative move is sent at playback, covered or not: a
    /// position may cover a move a game holding the pointer needs, and the
    /// compositor ignores the absolute move then (W01 decision 15).
    pub fn sends_every_move(self) -> bool {
        self == PointerSource::Hyprland
    }

    /// `point` on the backends' range across `layout`: from the middle of
    /// its pixel for a floored position, as reported otherwise.
    pub fn absolute(self, layout: &Layout, point: Point) -> AbsolutePosition {
        let floored = self == PointerSource::Hyprland;
        let on = layout
            .outputs()
            .iter()
            .find(|output| output.contains(point))
            .filter(|_| floored);
        match on {
            Some(output) => layout.absolute_at(
                f64::from(point.x) + 0.5,
                f64::from(point.y) + 0.5,
                output.rect(),
            ),
            None => layout.absolute(point),
        }
    }
}
