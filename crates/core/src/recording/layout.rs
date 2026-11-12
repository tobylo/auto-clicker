//! Where the pointer was: a point on the screens, and the screen layout it
//! lies on.
//!
//! Both are in whole logical pixels, as KWin reports them (decision 0001).
//! A [`Layout`] is only ever built through [`Layout::new`], which checks it,
//! so a layout from a file and one from the compositor are the same type
//! with the same rules: one to [`MAX_OUTPUTS`] outputs, each of positive
//! area, whose bounding box starts at (0, 0). One that arrives over the
//! settings window's socket or in the config file is read through
//! [`Layout::new`] too: it deserialises only through [`OutputSpec`].

use serde::{Deserialize, Serialize};

use crate::input::AbsolutePosition;

/// The most outputs a layout may have.
pub const MAX_OUTPUTS: usize = 16;

/// A point on the screens, in whole logical pixels from the layout's
/// top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: u32,
    pub y: u32,
}

/// A rectangle in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// One output (a screen): its rectangle in logical pixels and its scale in
/// thousandths (1.25 is 1250). The scale is stored, but whether positions
/// still fit a layout never depends on it ([`Layout::same_screens`],
/// [`Layout::keeps`]): logical positions do not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Output {
    rect: Rect,
    scale_milli: u32,
}

impl Output {
    /// An output covering `rect`, at `scale_milli`.
    ///
    /// # Errors
    /// A side or the scale is zero, or the rectangle reaches past
    /// `u32::MAX`.
    pub fn new(rect: Rect, scale_milli: u32) -> Result<Output, &'static str> {
        if rect.width == 0 || rect.height == 0 {
            return Err("a screen of the layout has no area");
        }
        if scale_milli == 0 {
            return Err("a screen of the layout has no scale");
        }
        if rect.x.checked_add(rect.width).is_none() || rect.y.checked_add(rect.height).is_none() {
            return Err("a screen of the layout reaches too far");
        }
        Ok(Output { rect, scale_milli })
    }

    pub fn rect(self) -> Rect {
        self.rect
    }

    pub fn scale_milli(self) -> u32 {
        self.scale_milli
    }

    /// The right edge, exclusive. [`Output::new`] checked that it fits.
    fn right(self) -> u32 {
        self.rect.x.saturating_add(self.rect.width)
    }

    /// The bottom edge, exclusive.
    fn bottom(self) -> u32 {
        self.rect.y.saturating_add(self.rect.height)
    }

    /// Whether `point` is on this output; its right and bottom edges are
    /// the next output's.
    pub fn contains(self, point: Point) -> bool {
        self.rect.contains(point)
    }
}

impl Rect {
    /// Whether `point` is inside, up to the last pixel before the right
    /// and bottom edges.
    pub fn contains(self, point: Point) -> bool {
        let right = self.x.saturating_add(self.width);
        let bottom = self.y.saturating_add(self.height);
        (self.x..right).contains(&point.x) && (self.y..bottom).contains(&point.y)
    }
}

/// One output as the config file and the settings window's socket spell
/// it, checked by [`Output::new`] on its way in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutputSpec {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub scale_milli: u32,
}

impl From<Output> for OutputSpec {
    fn from(output: Output) -> OutputSpec {
        let Rect {
            x,
            y,
            width,
            height,
        } = output.rect;
        OutputSpec {
            x,
            y,
            width,
            height,
            scale_milli: output.scale_milli,
        }
    }
}

impl TryFrom<OutputSpec> for Output {
    type Error = &'static str;

    fn try_from(spec: OutputSpec) -> Result<Output, &'static str> {
        let rect = Rect {
            x: spec.x,
            y: spec.y,
            width: spec.width,
            height: spec.height,
        };
        Output::new(rect, spec.scale_milli)
    }
}

/// A checked screen layout: the outputs, in a fixed order, so two reports of
/// the same screens compare equal whatever order they came in.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(try_from = "Vec<OutputSpec>", into = "Vec<OutputSpec>")]
pub struct Layout {
    outputs: Vec<Output>,
}

impl Layout {
    /// The layout of `outputs`. Outputs may overlap (mirrored screens).
    ///
    /// # Errors
    /// No outputs, more than [`MAX_OUTPUTS`], or a bounding box that does
    /// not start at (0, 0).
    pub fn new(mut outputs: Vec<Output>) -> Result<Layout, &'static str> {
        if outputs.is_empty() {
            return Err("the layout has no screens");
        }
        if outputs.len() > MAX_OUTPUTS {
            return Err("the layout has more than 16 screens");
        }
        let left = outputs.iter().map(|output| output.rect.x).min();
        let top = outputs.iter().map(|output| output.rect.y).min();
        if left != Some(0) || top != Some(0) {
            return Err("the layout does not start at the top-left corner");
        }
        outputs.sort_unstable();
        Ok(Layout { outputs })
    }

    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// Whether `point` lies on an output.
    pub fn holds(&self, point: Point) -> bool {
        self.outputs.iter().any(|output| output.contains(point))
    }

    /// Whether every output in `used` is here with the same place and size.
    pub fn keeps(&self, used: &[Output]) -> bool {
        used.iter()
            .all(|wanted| self.outputs.iter().any(|output| output.rect == wanted.rect))
    }

    /// Whether `other` has the same screens in the same places and sizes,
    /// whatever their scales: positions mean the same on both.
    pub fn same_screens(&self, other: &Layout) -> bool {
        let rects = |layout: &Layout| {
            layout
                .outputs
                .iter()
                .map(|output| output.rect)
                .collect::<Vec<_>>()
        };
        rects(self) == rects(other)
    }

    /// The bounding box's width and height, both at least 1.
    fn size(&self) -> (u32, u32) {
        let width = self.outputs.iter().map(|output| output.right()).max();
        let height = self.outputs.iter().map(|output| output.bottom()).max();
        (width.unwrap_or(1).max(1), height.unwrap_or(1).max(1))
    }

    /// `point` on the backends' normalised range across this layout:
    /// `clamp(round(x * 65536 / W), 0, 65535)`, likewise y with H
    /// (decision 0001).
    pub fn absolute(&self, point: Point) -> AbsolutePosition {
        let (width, height) = self.size();
        AbsolutePosition {
            x: normalised(point.x, width),
            y: normalised(point.y, height),
        }
    }
}

impl TryFrom<Vec<OutputSpec>> for Layout {
    type Error = &'static str;

    fn try_from(specs: Vec<OutputSpec>) -> Result<Layout, &'static str> {
        if specs.len() > MAX_OUTPUTS {
            return Err("the layout has more than 16 screens");
        }
        let outputs = specs.into_iter().map(Output::try_from);
        Layout::new(outputs.collect::<Result<_, _>>()?)
    }
}

impl From<Layout> for Vec<OutputSpec> {
    fn from(layout: Layout) -> Vec<OutputSpec> {
        layout.outputs.into_iter().map(OutputSpec::from).collect()
    }
}

/// `round(value * 65536 / size)`, at most 65535. In `u64` it cannot
/// overflow: both are below 2^32.
fn normalised(value: u32, size: u32) -> u16 {
    let size = u64::from(size.max(1));
    let scaled = (u64::from(value) * 65_536 + size / 2) / size;
    u16::try_from(scaled).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn output(x: u32, y: u32, width: u32, height: u32) -> Output {
        Output::new(rect(x, y, width, height), 1000).unwrap()
    }

    /// The laptop panel with DP-1 to its right (decision 0001).
    fn two_screens() -> Layout {
        Layout::new(vec![output(2048, 0, 3440, 1440), output(0, 0, 2048, 1280)]).unwrap()
    }

    #[test]
    fn an_output_needs_area_a_scale_and_room() {
        let cases = [
            (rect(0, 0, 0, 10), 1000, false),
            (rect(0, 0, 10, 0), 1000, false),
            (rect(0, 0, 10, 10), 0, false),
            (rect(u32::MAX, 0, 1, 10), 1000, false),
            (rect(0, u32::MAX, 10, 1), 1000, false),
            (rect(u32::MAX - 1, u32::MAX - 1, 1, 1), 1, true),
        ];
        for (area, scale, valid) in cases {
            assert_eq!(Output::new(area, scale).is_ok(), valid, "{area:?} {scale}");
        }
    }

    #[test]
    fn a_layout_has_one_to_sixteen_outputs_from_the_top_left() {
        let cases = [
            (Vec::new(), false),
            (vec![output(1, 0, 10, 10)], false),
            (vec![output(0, 1, 10, 10)], false),
            (vec![output(0, 0, 10, 10); MAX_OUTPUTS], true),
            (vec![output(0, 0, 10, 10); MAX_OUTPUTS + 1], false),
            // The corner may come from two outputs; mirrored ones overlap.
            (vec![output(0, 5, 10, 10), output(5, 0, 10, 10)], true),
        ];
        for (outputs, valid) in cases {
            assert_eq!(Layout::new(outputs.clone()).is_ok(), valid, "{outputs:?}");
        }
    }

    #[test]
    fn the_order_outputs_come_in_does_not_matter() {
        let other = Layout::new(vec![output(0, 0, 2048, 1280), output(2048, 0, 3440, 1440)]);
        assert_eq!(other.unwrap(), two_screens());
    }

    #[test]
    fn a_point_lies_on_an_output_up_to_its_last_pixel() {
        let layout = two_screens();
        let cases = [
            ((0, 0), true),
            ((2047, 1279), true),
            ((2048, 1439), true),
            ((5487, 0), true),
            // Below the panel, inside the bounding box: on no output.
            ((2047, 1280), false),
            ((5488, 0), false),
        ];
        for ((x, y), on) in cases {
            assert_eq!(layout.holds(Point { x, y }), on, "{x},{y}");
        }
    }

    #[test]
    fn the_same_screens_compare_rectangles_and_not_scale() {
        let layout = two_screens();
        let rescaled = Layout::new(vec![
            Output::new(rect(0, 0, 2048, 1280), 1250).unwrap(),
            Output::new(rect(2048, 0, 3440, 1440), 2000).unwrap(),
        ])
        .unwrap();
        assert!(layout.same_screens(&rescaled));
        let cases = [
            Layout::new(vec![output(0, 0, 2048, 1280)]).unwrap(),
            Layout::new(vec![output(0, 0, 2048, 1280), output(2048, 0, 3440, 1441)]).unwrap(),
            Layout::new(vec![
                output(0, 0, 2048, 1280),
                output(2048, 0, 3440, 1440),
                output(0, 1280, 10, 10),
            ])
            .unwrap(),
        ];
        for other in cases {
            assert!(!layout.same_screens(&other), "{other:?}");
        }
    }

    #[test]
    fn keeping_compares_rectangles_and_not_scale() {
        let layout = two_screens();
        let panel = output(0, 0, 2048, 1280);
        let cases = [
            (vec![panel], true),
            (vec![Output::new(panel.rect(), 1250).unwrap()], true),
            (vec![], true),
            (vec![output(0, 0, 2048, 1281)], false),
            (vec![output(1, 0, 2048, 1280)], false),
            (vec![panel, output(0, 1280, 100, 100)], false),
        ];
        for (used, kept) in cases {
            assert_eq!(layout.keeps(&used), kept, "{used:?}");
        }
    }

    #[test]
    fn positions_map_onto_the_whole_layout_by_the_formula() {
        let layout = two_screens(); // 5488 x 1440
        let cases = [
            ((0, 0), (0, 0)),
            // round(2744 * 65536 / 5488) = 32768; round(720 * 65536 / 1440) = 32768.
            ((2744, 720), (32768, 32768)),
            // round(1 * 65536 / 5488) = round(11.94) = 12; 1 / 1440 gives 45.51.
            ((1, 1), (12, 46)),
            // The last pixel; past the edge clamps.
            ((5487, 1439), (65524, 65490)),
            ((u32::MAX, u32::MAX), (65535, 65535)),
        ];
        for ((x, y), (to_x, to_y)) in cases {
            let expected = AbsolutePosition { x: to_x, y: to_y };
            assert_eq!(layout.absolute(Point { x, y }), expected, "{x},{y}");
        }
    }

    #[test]
    fn a_layout_reads_back_as_written_and_only_when_valid() {
        let layout = Layout::new(vec![
            Output::new(rect(0, 0, 2048, 1280), 1250).unwrap(),
            Output::new(rect(2048, 0, 2293, 960), 1500).unwrap(),
        ])
        .unwrap();
        let json = serde_json::to_string(&layout).unwrap();
        assert_eq!(
            json,
            r#"[{"x":0,"y":0,"width":2048,"height":1280,"scale_milli":1250},{"x":2048,"y":0,"width":2293,"height":960,"scale_milli":1500}]"#
        );
        assert_eq!(serde_json::from_str::<Layout>(&json).unwrap(), layout);
        let one = r#"{"x":0,"y":0,"width":10,"height":10,"scale_milli":1000}"#;
        let many = format!("[{}]", vec![one; MAX_OUTPUTS + 1].join(","));
        let invalid = [
            "[]".to_owned(),
            r#"[{"x":1,"y":0,"width":10,"height":10,"scale_milli":1000}]"#.to_owned(),
            r#"[{"x":0,"y":0,"width":0,"height":10,"scale_milli":1000}]"#.to_owned(),
            r#"[{"x":0,"y":0,"width":10,"height":10,"scale_milli":0}]"#.to_owned(),
            r#"[{"x":0,"y":0,"width":10,"height":10,"scale_milli":1000,"name":"DP-1"}]"#.to_owned(),
            many,
        ];
        for json in invalid {
            assert!(serde_json::from_str::<Layout>(&json).is_err(), "{json}");
        }
    }

    #[test]
    fn the_formula_rounds_halves_up() {
        // 1 * 65536 / 131072 = 0.5.
        let cases = [((1, 131_072), 1), ((0, 1), 0), ((1, 1), u16::MAX)];
        for ((value, size), expected) in cases {
            assert_eq!(normalised(value, size), expected, "{value}/{size}");
        }
    }
}
