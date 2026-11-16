//! Where a cursor session's position lies on the layout (W02).
//!
//! niri reports the pointer on each output in physical pixels, from the
//! output's top-left corner, in its displayed orientation: the pointer's
//! logical offset times the output's scale, rounded (decision 0004, "niri
//! cursor sessions"). The display tells a client no fractional scale, only
//! the current mode and the logical size xdg-output gives, rounded from the
//! mode over the scale. niri's scales are multiples of 1/120, so the scale
//! is the one multiple that gives back both logical sides; when none or
//! several do, the mode over the logical size stands in, at most half a
//! logical pixel off across the output. The offset over the scale, rounded
//! to the nearest whole logical pixel and kept inside its output, is the
//! position stored (version 2, as KWin's).

use auto_clicker_core::recording::Point;

/// The steps of niri's scales: 1/120.
const STEPS: u64 = 120;

/// How far from the mode over the logical size a scale is looked for, in
/// steps: further than rounding the logical size can move it.
const SEARCH: u64 = 3;

/// An output as a settled layout has it: its logical place on the layout,
/// already moved by the layout's corner, its logical size, and its current
/// mode as displayed (turned a quarter, the mode's width is its height).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Place {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// `None` without a current mode: no scale, so no position.
    pub(crate) physical: Option<(u32, u32)>,
}

impl Place {
    /// The position `(x, y)` physical pixels from the output's corner, on
    /// the layout; `None` when it lies outside the output (a session
    /// reports while any of the cursor's image overlaps its output, so
    /// that one is the neighbour's) or the output has no mode.
    pub(crate) fn point(&self, x: i32, y: i32) -> Option<Point> {
        let (physical_width, physical_height) = self.physical?;
        let x = u32::try_from(x).ok().filter(|&x| x < physical_width)?;
        let y = u32::try_from(y).ok().filter(|&y| y < physical_height)?;
        let scale = Scale::of(physical_width, self.width, physical_height, self.height)?;
        let (across, down) = scale.logical(x, y)?;
        Some(Point {
            x: self.x.checked_add(across.min(self.width.checked_sub(1)?))?,
            y: self.y.checked_add(down.min(self.height.checked_sub(1)?))?,
        })
    }
}

/// How physical pixels become logical ones on one output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scale {
    /// niri's scale, `steps` / 120.
    Steps(u64),
    /// The mode over the logical size on each axis: physical, logical.
    Ratio { x: (u64, u64), y: (u64, u64) },
}

impl Scale {
    /// The scale of an output whose mode is `physical_width` x
    /// `physical_height` and logical size `width` x `height`; `None` for an
    /// empty one.
    fn of(physical_width: u32, width: u32, physical_height: u32, height: u32) -> Option<Scale> {
        let sides = [
            (u64::from(physical_width), u64::from(width)),
            (u64::from(physical_height), u64::from(height)),
        ];
        if sides
            .iter()
            .any(|&(physical, logical)| physical == 0 || logical == 0)
        {
            return None;
        }
        let [x, y] = sides;
        let near = rounded(x.0.checked_mul(STEPS)?, x.1)?;
        let low = near.saturating_sub(SEARCH).max(1);
        let high = near.checked_add(SEARCH)?;
        let mut fits = (low..=high).filter(|&steps| {
            sides
                .iter()
                .all(|&(physical, logical)| gives(physical, steps) == Some(logical))
        });
        match (fits.next(), fits.next()) {
            (Some(steps), None) => Some(Scale::Steps(steps)),
            _ => Some(Scale::Ratio { x, y }),
        }
    }

    /// The physical offset `(x, y)` in whole logical pixels, rounded.
    fn logical(self, x: u32, y: u32) -> Option<(u32, u32)> {
        let (x, y) = (u64::from(x), u64::from(y));
        let (across, down) = match self {
            Scale::Steps(steps) => (
                rounded(x.checked_mul(STEPS)?, steps)?,
                rounded(y.checked_mul(STEPS)?, steps)?,
            ),
            Scale::Ratio {
                x: (physical_x, logical_x),
                y: (physical_y, logical_y),
            } => (
                rounded(x.checked_mul(logical_x)?, physical_x)?,
                rounded(y.checked_mul(logical_y)?, physical_y)?,
            ),
        };
        Some((u32::try_from(across).ok()?, u32::try_from(down).ok()?))
    }
}

/// The logical size `physical` pixels give at `steps` / 120, rounded as the
/// display rounds it.
fn gives(physical: u64, steps: u64) -> Option<u64> {
    rounded(physical.checked_mul(STEPS)?, steps)
}

/// `numerator` / `denominator`, rounded half up; `None` over zero.
fn rounded(numerator: u64, denominator: u64) -> Option<u64> {
    numerator
        .checked_mul(2)?
        .checked_add(denominator)?
        .checked_div(denominator.checked_mul(2)?)
}

#[cfg(test)]
mod tests;
