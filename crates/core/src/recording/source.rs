//! What reported a recording's positions, and how that decides the way
//! they play (W01 brief, "Settled by W01d").

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
}

impl PointerSource {
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
