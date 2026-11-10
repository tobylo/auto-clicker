//! Where an [`AbsolutePosition`] lands on the screens the compositor reports.
//!
//! libei's absolute pointer takes logical pixels on the whole output layout,
//! and the compositor reports one region per output (X02: `0,0 2048x1280`
//! and `2048,0 3440x1440` for two monitors side by side). The trait's
//! normalised 0..=65535 stretches over the regions' bounding box, as uinput
//! stretches it over every output, so a point means the same on both
//! backends (map D3: playback stores `round(x * 65536 / W)`). A point in the
//! box but on no output moves to the nearest point on one, as the compositor
//! does for uinput.

use auto_clicker_core::input::AbsolutePosition;

/// One output's rectangle, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl Region {
    fn left(self) -> f64 {
        f64::from(self.x)
    }

    fn top(self) -> f64 {
        f64::from(self.y)
    }

    /// The right edge, exclusive. Computed in `f64`, so it cannot overflow.
    fn right(self) -> f64 {
        self.left() + f64::from(self.width)
    }

    fn bottom(self) -> f64 {
        self.top() + f64::from(self.height)
    }

    /// The point of this region nearest to `(x, y)`: the point itself when
    /// it is inside. The last pixel's left edge is the farthest a clamp
    /// goes, so the point stays inside the region.
    fn nearest(self, (x, y): (f64, f64)) -> (f64, f64) {
        let last_x = (self.right() - 1.0).max(self.left());
        let last_y = (self.bottom() - 1.0).max(self.top());
        let inside_x = x >= self.left() && x < self.right();
        let inside_y = y >= self.top() && y < self.bottom();
        (
            if inside_x {
                x
            } else {
                x.clamp(self.left(), last_x)
            },
            if inside_y {
                y
            } else {
                y.clamp(self.top(), last_y)
            },
        )
    }
}

/// The outputs an absolute pointer can reach.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    /// Every region with an area; never empty.
    regions: Vec<Region>,
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}

impl Layout {
    /// The layout over `regions`; `None` when none has an area.
    pub(crate) fn new(regions: impl IntoIterator<Item = Region>) -> Option<Layout> {
        let regions: Vec<Region> = regions
            .into_iter()
            .filter(|region| region.width > 0 && region.height > 0)
            .collect();
        let left = regions.iter().map(|r| r.left()).reduce(f64::min)?;
        let top = regions.iter().map(|r| r.top()).reduce(f64::min)?;
        let right = regions.iter().map(|r| r.right()).reduce(f64::max)?;
        let bottom = regions.iter().map(|r| r.bottom()).reduce(f64::max)?;
        Some(Layout {
            regions,
            left,
            top,
            width: right - left,
            height: bottom - top,
        })
    }

    /// The outputs, for the device test.
    #[cfg(test)]
    pub(crate) fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// The position that names the logical-pixel point `(x, y)`, as
    /// playback stores it (D3): the inverse of [`Layout::point`].
    #[cfg(test)]
    pub(crate) fn position_of(&self, x: f64, y: f64) -> AbsolutePosition {
        let axis = |offset: f64, size: f64| {
            let steps = ((offset * STEPS / size).round()).clamp(0.0, f64::from(u16::MAX));
            // Clamped to u16's range just above.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let steps = steps as u16;
            steps
        };
        AbsolutePosition {
            x: axis(x - self.left, self.width),
            y: axis(y - self.top, self.height),
        }
    }

    /// The logical-pixel point `position` names, on an output.
    pub(crate) fn point(&self, position: AbsolutePosition) -> (f32, f32) {
        let scaled = (
            self.left + f64::from(position.x) * self.width / STEPS,
            self.top + f64::from(position.y) * self.height / STEPS,
        );
        let (x, y) = self
            .regions
            .iter()
            .map(|region| region.nearest(scaled))
            .min_by(|a, b| distance(*a, scaled).total_cmp(&distance(*b, scaled)))
            // `new` keeps at least one region.
            .unwrap_or(scaled);
        // Logical pixels fit in f32 with room to spare: f32 is exact to 2^24.
        #[allow(clippy::cast_possible_truncation)]
        (x as f32, y as f32)
    }
}

/// The normalised range's steps per axis: 65535 maps just inside the far
/// edge, as D3's `round(x * 65536 / W)` does in reverse.
const STEPS: f64 = 65536.0;

fn distance((ax, ay): (f64, f64), (bx, by): (f64, f64)) -> f64 {
    (ax - bx).powi(2) + (ay - by).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(x: u32, y: u32, width: u32, height: u32) -> Region {
        Region {
            x,
            y,
            width,
            height,
        }
    }

    /// X02's laptop: eDP-1 beside the taller DP-1.
    fn two_monitors() -> Layout {
        Layout::new([region(0, 0, 2048, 1280), region(2048, 0, 3440, 1440)]).unwrap()
    }

    fn at(x: u16, y: u16) -> AbsolutePosition {
        AbsolutePosition { x, y }
    }

    #[test]
    fn one_output_spans_the_whole_range() {
        let layout = Layout::new([region(0, 0, 2048, 1280)]).unwrap();
        assert_eq!(layout.point(at(0, 0)), (0.0, 0.0));
        assert_eq!(layout.point(at(32768, 32768)), (1024.0, 640.0));
        assert_eq!(layout.point(at(65535, 65535)), (2_047.968_8, 1_279.980_5));
    }

    #[test]
    fn the_range_spans_the_bounding_box_of_every_output() {
        let layout = two_monitors();
        // Across both: 5488 wide, 1440 high.
        assert_eq!(layout.point(at(32768, 32768)), (2744.0, 720.0));
        assert_eq!(layout.point(at(65535, 0)), (5487.916, 0.0));
        // D3's mapping in reverse: x = 3000 px was stored as round(3000 * 65536 / 5488).
        let stored = (3000.0_f64 * 65536.0 / 5488.0).round() as u16;
        let (x, _) = layout.point(at(stored, 0));
        assert!((x - 3000.0).abs() < 0.1, "{x}");
    }

    #[test]
    fn a_point_on_no_output_moves_to_the_nearest_one() {
        let layout = two_monitors();
        // Below the shorter laptop screen: up to its last row.
        assert_eq!(layout.point(at(0, 65535)), (0.0, 1279.0));
        assert_eq!(layout.point(at(16384, 62000)), (1372.0, 1279.0));
        // Just left of the taller monitor, in its extra rows: onto its first column.
        let (x, y) = layout.point(at(24400, 61000));
        assert_eq!(x, 2048.0);
        assert!(y > 1280.0 && y < 1440.0, "{y}");
    }

    #[test]
    fn outputs_need_not_start_at_the_origin() {
        let layout = Layout::new([region(100, 50, 200, 100)]).unwrap();
        assert_eq!(layout.point(at(0, 0)), (100.0, 50.0));
        assert_eq!(layout.point(at(32768, 32768)), (200.0, 100.0));
    }

    #[test]
    fn regions_without_an_area_are_left_out() {
        assert_eq!(Layout::new([]), None);
        assert_eq!(
            Layout::new([region(0, 0, 0, 100), region(5, 5, 100, 0)]),
            None
        );
        let layout = Layout::new([region(0, 0, 0, 900), region(10, 10, 100, 100)]).unwrap();
        assert_eq!(layout.point(at(0, 0)), (10.0, 10.0));
    }

    #[test]
    fn huge_regions_do_not_overflow() {
        let layout = Layout::new([region(u32::MAX, u32::MAX, u32::MAX, u32::MAX)]).unwrap();
        let (x, y) = layout.point(at(65535, 65535));
        assert!(x.is_finite() && y.is_finite());
    }
}
