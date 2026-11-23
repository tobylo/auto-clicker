//! A tile with rounded corners, as the tray icon and the on-screen badge
//! both draw it: each pixel sampled 4 by 4 times, so the corners and the
//! marks on the tile have smooth edges. What each caller packs a pixel into
//! is its own: the tray wants straight ARGB, a Wayland buffer premultiplied
//! alpha.

use crate::digits::{Point, Rgb};

/// Samples per pixel along each axis, for smooth edges.
pub const SAMPLES: u16 = 4;

/// The corner radius, as a share of the side, as in the themed icons.
pub const RADIUS: f32 = 14.0 / 64.0;

/// A square tile `side` pixels wide, its top left corner at the origin.
#[derive(Clone, Copy, Debug)]
pub struct Tile {
    side: f32,
    radius: f32,
}

impl Tile {
    pub fn new(side: f32) -> Tile {
        Tile {
            side,
            radius: RADIUS * side,
        }
    }

    /// Whether `point` lies on the tile, inside its rounded corners.
    pub fn contains(&self, Point { x, y }: Point) -> bool {
        let far = self.side - self.radius;
        let dx = (self.radius - x).max(x - far).max(0.0);
        let dy = (self.radius - y).max(y - far).max(0.0);
        dx * dx + dy * dy <= self.radius * self.radius
    }
}

/// A pixel as its samples found it: how much of it is covered, and the
/// average colour of what covers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pixel {
    /// 0 for none, 255 for all of it.
    pub alpha: u8,
    pub colour: Rgb,
}

impl Pixel {
    /// Straight ARGB, as the tray icon's pixmaps want it.
    pub fn argb(self) -> [u8; 4] {
        let Rgb(r, g, b) = self.colour;
        [self.alpha, r, g, b]
    }

    /// Premultiplied ARGB8888 in memory order (blue, green, red, alpha),
    /// as a Wayland shared-memory buffer wants it.
    pub fn premultiplied_bgra(self) -> [u8; 4] {
        let alpha = u16::from(self.alpha);
        // At most 255 * 255 / 255: always a byte.
        let times =
            |channel: u8| u8::try_from((u16::from(channel) * alpha + 127) / 255).unwrap_or(u8::MAX);
        let Rgb(r, g, b) = self.colour;
        [times(b), times(g), times(r), self.alpha]
    }
}

/// The pixel whose top left corner is `corner`, from the colour at each of
/// its samples: `None` where nothing covers the sample.
pub fn sample(corner: Point, colour: impl Fn(Point) -> Option<Rgb>) -> Pixel {
    let offsets = (0..SAMPLES).map(|i| (f32::from(i) + 0.5) / f32::from(SAMPLES));
    let points = offsets.clone().flat_map(|dy| {
        offsets.clone().map(move |dx| Point {
            x: corner.x + dx,
            y: corner.y + dy,
        })
    });
    let mut samples = Samples::default();
    points
        .filter_map(colour)
        .for_each(|colour| samples.add(colour));
    samples.pixel()
}

/// A pixel's samples that something covers: how many, and their channels
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

    /// The alpha from how many samples are covered out of all, the colour
    /// from those that are.
    fn pixel(&self) -> Pixel {
        let all = u32::from(SAMPLES * SAMPLES);
        // Averages of bytes, and a share of 255: never past a byte.
        let byte = |value: u32| u8::try_from(value).unwrap_or(u8::MAX);
        match self.sum.map(|total| total.checked_div(self.count)) {
            [Some(r), Some(g), Some(b)] => Pixel {
                alpha: byte(self.count * 255 / all),
                colour: Rgb(byte(r), byte(g), byte(b)),
            },
            _ => Pixel {
                alpha: 0,
                colour: Rgb(0, 0, 0),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Rgb = Rgb(0xff, 0xff, 0xff);

    #[test]
    fn a_pixel_averages_its_samples_that_are_covered() {
        let mut samples = Samples::default();
        assert_eq!(samples.pixel().argb(), [0; 4]);
        for _ in 0..4 {
            samples.add(WHITE);
            samples.add(Rgb(0x01, 0x02, 0x03));
        }
        // Half the samples covered, half of those white.
        assert_eq!(samples.pixel().argb(), [0x7f, 0x80, 0x80, 0x81]);
        for _ in 0..8 {
            samples.add(WHITE);
        }
        assert_eq!(samples.pixel().alpha, 0xff);
    }

    #[test]
    fn the_tile_rounds_its_corners_on_a_circle() {
        // 64 px, so the radius is 14, as in the themed icons.
        let tile = Tile::new(64.0);
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
            assert_eq!(tile.contains(Point { x, y }), on, "({x}, {y})");
        }
    }

    #[test]
    fn a_covered_pixel_is_its_colour_and_an_edge_is_premultiplied() {
        let red = Rgb(0xef, 0x44, 0x44);
        let whole = sample(Point { x: 3.0, y: 3.0 }, |_| Some(red));
        assert_eq!(
            whole,
            Pixel {
                alpha: 0xff,
                colour: red
            }
        );
        assert_eq!(whole.premultiplied_bgra(), [0x44, 0x44, 0xef, 0xff]);
        assert_eq!(whole.argb(), [0xff, 0xef, 0x44, 0x44]);
        // The left half covered.
        let half = sample(Point { x: 0.0, y: 0.0 }, |p| (p.x < 0.5).then_some(red));
        assert_eq!(half.alpha, 0x7f);
        assert_eq!(half.colour, red);
        // Each channel scaled by the alpha, rounded.
        assert_eq!(half.premultiplied_bgra(), [0x22, 0x22, 0x77, 0x7f]);
        // The top half of a pixel lower down: its samples follow its row.
        let top = sample(Point { x: 0.0, y: 2.0 }, |p| (p.y < 2.5).then_some(red));
        assert_eq!(top.alpha, 0x7f);
        let none = sample(Point { x: 0.0, y: 0.0 }, |_| None);
        assert_eq!(none.premultiplied_bgra(), [0; 4]);
    }
}
