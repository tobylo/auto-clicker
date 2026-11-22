//! The tray icon during a countdown: the time left, drawn in code over a tile
//! in the job's colour. Plasma shows these pixmaps while the icon has no name.
//! The digits' shapes are core's ([`digits`]), which the on-screen display
//! draws too.
//!
//! Nothing here keeps state or wakes on its own: the pixmaps are drawn from
//! the view each time it changes, which a countdown does once a second.

use auto_clicker_core::controller::Activity;
use auto_clicker_core::digits::{self, Label, Point, Rect, Rgb};

/// The sizes Plasma picks from: the default panel's, and larger panels' or
/// scaled screens'.
const SIZES: [u16; 3] = [22, 32, 48];

/// Samples per pixel along each axis, for smooth edges.
const SAMPLES: u16 = 4;

/// Height of a character, as a share of the icon's side, when it fits.
const HEIGHT: f32 = 0.74;
/// The widest the label may be, as a share of the icon's side.
const WIDTH: f32 = 0.88;
/// The tile's corner radius, as a share of its side, as in the themed icons.
const RADIUS: f32 = 14.0 / 64.0;

/// The digits' colour.
const WHITE: Rgb = Rgb(0xff, 0xff, 0xff);

/// Whether `activity` is a countdown, which shows on the icon.
pub fn counts_down(activity: Activity) -> bool {
    digits::countdown(activity).is_some()
}

/// The icon's pixmaps for `activity`: one per size during a countdown, none
/// otherwise.
pub fn pixmaps(activity: Activity) -> Vec<ksni::Icon> {
    let Some((seconds, tile)) = digits::countdown(activity) else {
        return Vec::new();
    };
    let label = Label::of(seconds);
    SIZES
        .iter()
        .map(|&size| render(&label, tile, size))
        .collect()
}

/// The rectangles that draw `label` centred on an icon `side` pixels wide,
/// as large as fits and on whole pixels, so that the strokes stay sharp. A
/// single digit is no smaller than each of two.
fn layout(label: &Label, side: f32) -> Vec<Rect> {
    let height = (HEIGHT * side).min(WIDTH * side / label.span()).floor();
    let corner = Point {
        x: ((side - label.width(height)) / 2.0).floor(),
        y: ((side - height) / 2.0).floor(),
    };
    label.draw(corner, height)
}

/// What an icon shows: the label's strokes on a tile with rounded corners.
struct Scene {
    side: f32,
    radius: f32,
    lit: Vec<Rect>,
    tile: Rgb,
}

impl Scene {
    fn new(lit: Vec<Rect>, tile: Rgb, side: f32) -> Scene {
        Scene {
            side,
            radius: RADIUS * side,
            lit,
            tile,
        }
    }

    /// The colour at `point`, or `None` outside the tile's rounded corners.
    fn colour(&self, point: Point) -> Option<Rgb> {
        if !self.on_tile(point) {
            None
        } else if self.lit.iter().any(|rect| rect.contains(point)) {
            Some(WHITE)
        } else {
            Some(self.tile)
        }
    }

    fn on_tile(&self, Point { x, y }: Point) -> bool {
        let far = self.side - self.radius;
        let dx = (self.radius - x).max(x - far).max(0.0);
        let dy = (self.radius - y).max(y - far).max(0.0);
        dx * dx + dy * dy <= self.radius * self.radius
    }

    /// The pixel whose top left corner is `corner` as ARGB, its samples
    /// averaged.
    fn pixel(&self, corner: Point) -> [u8; 4] {
        let offsets = (0..SAMPLES).map(|i| (f32::from(i) + 0.5) / f32::from(SAMPLES));
        let points = offsets.clone().flat_map(|dy| {
            offsets.clone().map(move |dx| Point {
                x: corner.x + dx,
                y: corner.y + dy,
            })
        });
        let mut samples = Samples::default();
        points
            .filter_map(|point| self.colour(point))
            .for_each(|colour| samples.add(colour));
        samples.argb()
    }
}

/// A pixel's samples that fall on the tile: how many, and their channels
/// added up.
#[derive(Default)]
struct Samples {
    count: u32,
    sum: [u32; 3],
}

impl Samples {
    fn add(&mut self, Rgb(r, g, b): Rgb) {
        self.count += 1;
        for (total, channel) in self.sum.iter_mut().zip([r, g, b]) {
            *total += u32::from(channel);
        }
    }

    /// The pixel as ARGB: the alpha from how many samples fall on the tile
    /// out of all, the colour from those that do.
    fn argb(&self) -> [u8; 4] {
        let all = u32::from(SAMPLES * SAMPLES);
        // Averages of bytes, and a share of 255: never past a byte.
        let byte = |value: u32| u8::try_from(value).unwrap_or(u8::MAX);
        match self.sum.map(|total| total.checked_div(self.count)) {
            [Some(r), Some(g), Some(b)] => {
                [byte(self.count * 255 / all), byte(r), byte(g), byte(b)]
            }
            _ => [0; 4],
        }
    }
}

/// `label` on a tile in `tile`'s colour, `size` pixels square, as ARGB32 in
/// network byte order.
fn render(label: &Label, tile: Rgb, size: u16) -> ksni::Icon {
    let side = f32::from(size);
    let scene = Scene::new(layout(label, side), tile, side);
    let data = (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            scene.pixel(Point {
                x: f32::from(x),
                y: f32::from(y),
            })
        })
        .collect();
    ksni::Icon {
        width: i32::from(size),
        height: i32::from(size),
        data,
    }
}

#[cfg(test)]
mod tests {
    use auto_clicker_core::digits::{CLICKING, RECORDING};

    use super::*;

    /// The ARGB pixel at (x, y) of `icon`.
    fn at(icon: &ksni::Icon, x: i32, y: i32) -> [u8; 4] {
        let index = usize::try_from((y * icon.width + x) * 4).unwrap();
        icon.data[index..index + 4].try_into().unwrap()
    }

    fn opaque(Rgb(r, g, b): Rgb) -> [u8; 4] {
        [0xff, r, g, b]
    }

    /// The smallest rectangle around `rects`.
    fn bounds(rects: &[Rect]) -> Rect {
        Rect {
            x: rects.iter().map(|r| r.x).fold(f32::MAX, f32::min),
            y: rects.iter().map(|r| r.y).fold(f32::MAX, f32::min),
            right: rects.iter().map(|r| r.right).fold(f32::MIN, f32::max),
            bottom: rects.iter().map(|r| r.bottom).fold(f32::MIN, f32::max),
        }
    }

    #[test]
    fn only_a_countdown_draws_pixmaps_in_its_jobs_colour() {
        use Activity::*;
        for (activity, tile) in [
            (StartingIn(5), CLICKING),
            (PlayingIn(5), CLICKING),
            (RecordingIn(5), RECORDING),
        ] {
            assert!(counts_down(activity), "{activity:?}");
            let icons = pixmaps(activity);
            let sizes: Vec<(i32, i32)> = icons.iter().map(|i| (i.width, i.height)).collect();
            assert_eq!(sizes, [(22, 22), (32, 32), (48, 48)], "{activity:?}");
            for icon in &icons {
                let length = usize::try_from(icon.width * icon.height * 4).unwrap();
                assert_eq!(icon.data.len(), length, "{activity:?}");
                // The tile's left edge, halfway down: clear of the
                // corners and the digits.
                assert_eq!(at(icon, 1, icon.height / 2), opaque(tile), "{activity:?}");
            }
        }
        for activity in [
            Idle,
            WaitingForRelease,
            Opening,
            Clicking,
            Recording,
            PlayWaitingForRelease,
            PlayOpening,
            Playing,
        ] {
            assert!(!counts_down(activity), "{activity:?}");
            assert!(pixmaps(activity).is_empty(), "{activity:?}");
        }
    }

    #[test]
    fn the_tile_has_rounded_corners_with_smooth_edges() {
        for size in SIZES {
            let icon = render(&Label::of(8), CLICKING, size);
            let last = icon.width - 1;
            for (x, y) in [(0, 0), (last, 0), (0, last), (last, last)] {
                assert_eq!(at(&icon, x, y)[0], 0, "{size} px corner ({x}, {y})");
            }
            // Partly on the tile, where the corner's curve passes.
            let radius = (RADIUS * f32::from(size)) as i32;
            let corner = (0..radius).flat_map(|y| (0..radius).map(move |x| (x, y)));
            let partial = corner
                .map(|(x, y)| at(&icon, x, y)[0])
                .filter(|&alpha| alpha > 0 && alpha < 0xff)
                .count();
            assert!(partial >= 2, "{size} px: {partial} smooth pixels");
            assert_eq!(at(&icon, radius, 0)[0], 0xff, "{size} px top edge");
        }
    }

    #[test]
    fn a_pixel_averages_its_samples_on_the_tile() {
        let mut samples = Samples::default();
        assert_eq!(samples.argb(), [0; 4]);
        for _ in 0..4 {
            samples.add(WHITE);
            samples.add(Rgb(0x01, 0x02, 0x03));
        }
        // Half the samples on the tile, half of those white.
        assert_eq!(samples.argb(), [0x7f, 0x80, 0x80, 0x81]);
        for _ in 0..8 {
            samples.add(WHITE);
        }
        assert_eq!(samples.argb()[0], 0xff);
    }

    #[test]
    fn the_tile_rounds_its_corners_on_a_circle() {
        // 64 px, so the radius is 14, as in the themed icons.
        let scene = Scene::new(Vec::new(), CLICKING, 64.0);
        for (x, y, on) in [
            (0.5, 0.5, false),
            (2.0, 2.0, false),
            (5.0, 5.0, true),
            (14.0, 0.5, true),
            (0.5, 14.0, true),
            (62.0, 62.0, false),
            (59.0, 59.0, true),
            (32.0, 32.0, true),
        ] {
            assert_eq!(scene.on_tile(Point { x, y }), on, "({x}, {y})");
        }
    }

    #[test]
    fn the_strokes_are_white_on_the_tile() {
        for size in SIZES {
            for seconds in [1, 8, 300] {
                let label = Label::of(seconds);
                let rects = layout(&label, f32::from(size));
                let icon = render(&label, RECORDING, size);
                for rect in &rects {
                    // A stroke's top left pixel, wholly inside it.
                    let (x, y) = (rect.x as i32, rect.y as i32);
                    assert_eq!(at(&icon, x, y), opaque(WHITE), "{seconds} at {size} px");
                }
                // Left of the label, halfway down: the tile.
                let left = bounds(&rects).x as i32 - 1;
                let shown = at(&icon, left, icon.height / 2);
                assert_eq!(shown, opaque(RECORDING), "{seconds} at {size} px");
            }
        }
    }

    #[test]
    fn the_label_fills_most_of_the_icon_centred_on_whole_pixels() {
        for size in SIZES {
            for seconds in [7, 88, 300, 3600] {
                assert_centred(size, &Label::of(seconds));
            }
            // One digit stands no smaller than each of two, and two take
            // most of the icon's height.
            let side = f32::from(size);
            let height = |seconds| {
                let around = bounds(&layout(&Label::of(seconds), side));
                around.bottom - around.y
            };
            assert!(height(7) >= height(88), "{size} px");
            // Never the full height: the tile shows above and below.
            assert!(height(7) <= HEIGHT * side, "{size} px");
            assert!(height(88) >= 0.6 * side, "{size} px");
        }
    }

    /// Checks that `label` lies on the icon `size` pixels square, centred
    /// to the pixel, with its edges on whole pixels.
    fn assert_centred(size: u16, label: &Label) {
        let side = f32::from(size);
        let rects = layout(label, side);
        let shown = label.text();
        assert!(!rects.is_empty(), "{shown} at {size} px");
        let Rect {
            x: left,
            y: top,
            right,
            bottom,
        } = bounds(&rects);
        assert!(
            left >= 0.0 && right <= side && top >= 0.0 && bottom <= side,
            "{shown} at {size} px: {left}..{right}, {top}..{bottom}"
        );
        assert!((left - (side - right)).abs() <= 1.0, "{shown} at {size} px");
        assert!((top - (side - bottom)).abs() <= 1.0, "{shown} at {size} px");
        let whole = [left, right, top, bottom]
            .iter()
            .all(|edge| *edge == edge.round());
        assert!(whole, "{shown} at {size} px");
    }
}
