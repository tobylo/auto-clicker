//! The tray icon's pictures, drawn in code: during a countdown the time
//! left over a tile in the job's colour, on both systems; otherwise the
//! state's picture, which Windows draws here ([`picture`]) and Plasma takes
//! from the themed icons. The digits' shapes and the tile's sampling are
//! core's ([`digits`], [`tile`]), which the on-screen display draws too.
//!
//! Nothing here keeps state or wakes on its own: the pictures are drawn
//! from the view each time it changes, which a countdown does once a second.

mod picture;

use auto_clicker_core::controller::Activity;
use auto_clicker_core::digits::{self, Label, Point, Rect, Rgb, Size};
use auto_clicker_core::tile::{self, Pixel, Tile};

#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "Plasma shows the themed icons")
)]
pub use picture::draw;

/// Height of a character, as a share of the icon's side, when it fits.
const HEIGHT: f32 = 0.74;
/// The widest the label may be, as a share of the icon's side.
const WIDTH: f32 = 0.88;

/// The digits' colour.
const WHITE: Rgb = Rgb(0xff, 0xff, 0xff);

/// A square picture, `side` pixels wide: straight (not premultiplied)
/// RGBA, row by row from the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub side: u16,
    pub rgba: Vec<u8>,
}

impl Image {
    /// The picture `side` pixels square whose pixel at each top left
    /// corner `pixel` gives.
    fn sampled(side: u16, pixel: impl Fn(Point) -> Pixel) -> Image {
        let rgba = (0..side)
            .flat_map(|y| (0..side).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let Pixel { alpha, colour } = pixel(Point {
                    x: f32::from(x),
                    y: f32::from(y),
                });
                let Rgb(r, g, b) = colour;
                [r, g, b, alpha]
            })
            .collect();
        Image { side, rgba }
    }
}

/// The state's picture: what the icon shows when no countdown does. Plasma
/// names a themed icon for each; Windows draws them ([`draw`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    Idle,
    Clicking,
    Recording,
}

impl Picture {
    /// The picture for `activity`. A countdown shows the idle picture where
    /// the digits do not show (Plasma's tooltip).
    pub fn of(activity: Activity) -> Picture {
        match activity {
            Activity::Clicking | Activity::Playing => Picture::Clicking,
            Activity::Recording => Picture::Recording,
            Activity::Idle
            | Activity::StartingIn(_)
            | Activity::WaitingForRelease
            | Activity::Opening
            | Activity::RecordingIn(_)
            | Activity::PlayingIn(_)
            | Activity::PlayWaitingForRelease
            | Activity::PlayOpening => Picture::Idle,
        }
    }
}

/// The seconds left of `activity`'s countdown on a tile `side` pixels
/// square; none when it is no countdown.
pub fn countdown(activity: Activity, side: u16) -> Option<Image> {
    let (seconds, tile) = digits::countdown(activity)?;
    Some(render(&Label::of(seconds), tile, side))
}

/// What the icon shows for `activity`, `side` pixels square: the countdown,
/// or else the state's picture.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "Plasma shows the themed icons")
)]
pub fn image(activity: Activity, side: u16) -> Image {
    countdown(activity, side).unwrap_or_else(|| draw(Picture::of(activity), side))
}

/// The rectangles that draw `label` centred on an icon `side` pixels wide,
/// as large as fits and on whole pixels, so that the strokes stay sharp. A
/// single digit is no smaller than each of two.
fn layout(label: &Label, side: f32) -> Vec<Rect> {
    let height = (HEIGHT * side).min(WIDTH * side / label.span()).floor();
    let icon = Size {
        width: side,
        height: side,
    };
    label.centred(icon, height)
}

/// What an icon shows: the label's strokes on a tile with rounded corners.
struct Scene {
    shape: Tile,
    lit: Vec<Rect>,
    tile: Rgb,
}

impl Scene {
    fn new(lit: Vec<Rect>, tile: Rgb, side: f32) -> Scene {
        Scene {
            shape: Tile::new(side),
            lit,
            tile,
        }
    }

    /// The colour at `point`, or `None` outside the tile's rounded corners.
    fn colour(&self, point: Point) -> Option<Rgb> {
        if !self.shape.contains(point) {
            None
        } else if self.lit.iter().any(|rect| rect.contains(point)) {
            Some(WHITE)
        } else {
            Some(self.tile)
        }
    }
}

/// `label` on a tile in `tile`'s colour, `size` pixels square.
fn render(label: &Label, tile: Rgb, size: u16) -> Image {
    let side = f32::from(size);
    let scene = Scene::new(layout(label, side), tile, side);
    Image::sampled(size, |corner| {
        tile::sample(corner, |point| scene.colour(point))
    })
}

#[cfg(test)]
mod tests {
    use auto_clicker_core::digits::{CLICKING, RECORDING};
    use auto_clicker_core::tile::RADIUS;

    use super::*;

    /// The sizes drawn: Windows' small icon at 100 % and Plasma's.
    const SIZES: [u16; 4] = [16, 22, 32, 48];
    /// Where alpha is in a pixel.
    const ALPHA: usize = 3;

    /// The RGBA pixel at (x, y) of `icon`.
    pub fn at(icon: &Image, x: i32, y: i32) -> [u8; 4] {
        let index = usize::try_from((y * i32::from(icon.side) + x) * 4).unwrap();
        icon.rgba[index..index + 4].try_into().unwrap()
    }

    pub fn opaque(Rgb(r, g, b): Rgb) -> [u8; 4] {
        [r, g, b, 0xff]
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
    fn only_a_countdown_draws_digits_in_its_jobs_colour() {
        use Activity::*;
        for (activity, tile) in [
            (StartingIn(5), CLICKING),
            (PlayingIn(5), CLICKING),
            (RecordingIn(5), RECORDING),
        ] {
            for size in SIZES {
                let icon = countdown(activity, size).unwrap();
                assert_eq!(icon.side, size, "{activity:?}");
                assert_eq!(
                    icon.rgba.len(),
                    usize::from(size).pow(2) * 4,
                    "{activity:?}"
                );
                // The tile's left edge, halfway down: clear of the
                // corners and the digits.
                let half = i32::from(size) / 2;
                assert_eq!(at(&icon, 1, half), opaque(tile), "{activity:?}");
                assert_eq!(image(activity, size), icon, "{activity:?}");
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
            assert!(countdown(activity, 16).is_none(), "{activity:?}");
            let picture = draw(Picture::of(activity), 16);
            assert_eq!(image(activity, 16), picture, "{activity:?}");
        }
    }

    #[test]
    fn the_tile_has_rounded_corners_with_smooth_edges() {
        for size in SIZES {
            let icon = render(&Label::of(8), CLICKING, size);
            let last = i32::from(icon.side) - 1;
            for (x, y) in [(0, 0), (last, 0), (0, last), (last, last)] {
                assert_eq!(at(&icon, x, y)[ALPHA], 0, "{size} px corner ({x}, {y})");
            }
            // Partly on the tile, where the corner's curve passes.
            let radius = (RADIUS * f32::from(size)) as i32;
            let corner = (0..radius).flat_map(|y| (0..radius).map(move |x| (x, y)));
            let partial = corner
                .map(|(x, y)| at(&icon, x, y)[ALPHA])
                .filter(|&alpha| alpha > 0 && alpha < 0xff)
                .count();
            assert!(partial >= 2, "{size} px: {partial} smooth pixels");
            assert_eq!(at(&icon, radius, 0)[ALPHA], 0xff, "{size} px top edge");
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
                let shown = at(&icon, left, i32::from(icon.side) / 2);
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
