//! The pictures, drawn in code into ARGB8888 (premultiplied, little endian)
//! as the tray icon draws its digits: each pixel sampled 4 x 4 times.

use crate::glyphs::{self, Point, Rect};

/// Samples per pixel along each axis, as the tray icon's.
const SAMPLES: u16 = 4;
/// The number's characters, as a share of the screen's height.
const NUMBER_HEIGHT: f32 = 0.25;
/// The space around the number, as a share of its height.
const PAD: f32 = 0.25;
/// The badge's side, in logical px.
pub const BADGE: u32 = 40;
/// The pattern's size, in logical px.
const PATTERN: (u32, u32) = (400, 200);
/// The badge's tile: the clicking icon's indigo, see-through.
const TILE: [f32; 4] = [
    0x43 as f32 / 255.0,
    0x38 as f32 / 255.0,
    0xca as f32 / 255.0,
    0.8,
];
/// The countdown's digits: the job's colour (indigo; red for a recording),
/// opaque, with no backdrop.
const DIGITS: [f32; 4] = [
    0x43 as f32 / 255.0,
    0x38 as f32 / 255.0,
    0xca as f32 / 255.0,
    1.0,
];
/// The digits' outline, as a share of their height, so they read on light
/// and dark scenes alike.
const OUTLINE: f32 = 0.025;
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// What a surface shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    /// The countdown's seconds, in the middle of the screen.
    Number(u64),
    /// The badge, in the top left corner.
    Badge,
    /// One-pixel stripes, in the middle: sharpness at a fractional scale.
    Pattern,
}

/// Pixels in ARGB8888, `width * 4` bytes a row.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

/// The picture's size in logical px on a screen `screen` logical px large.
pub fn logical_size(picture: Picture, screen: (u32, u32)) -> (u32, u32) {
    match picture {
        Picture::Number(seconds) => {
            let height = (screen.1 as f32 * NUMBER_HEIGHT).round();
            let span = glyphs::span(&glyphs::label(seconds));
            let pad = (height * PAD).round();
            (
                (height * span + 2.0 * pad).round() as u32,
                (height + 2.0 * pad) as u32,
            )
        }
        Picture::Badge => (BADGE, BADGE),
        Picture::Pattern => PATTERN,
    }
}

/// `picture` drawn into a buffer `size` px large.
pub fn draw(picture: Picture, size: (u32, u32)) -> Image {
    match picture {
        // The digits and their outline are rectangles on whole pixels: each
        // pixel is wholly in or out of each, so one sample is exact.
        Picture::Number(seconds) => sampled(&Number::new(seconds, size), size, 1),
        Picture::Badge => sampled(&Badge::new(size), size, SAMPLES),
        Picture::Pattern => stripes(size),
    }
}

/// A picture as a colour at each point: premultiplied RGBA, 0 to 1.
trait Scene {
    fn colour(&self, point: Point) -> [f32; 4];
}

fn sampled(scene: &dyn Scene, (width, height): (u32, u32), samples: u16) -> Image {
    let mut bytes = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            bytes.extend(pixel(scene, x as f32, y as f32, samples));
        }
    }
    Image {
        width,
        height,
        bytes,
    }
}

/// The pixel whose top left corner is (x, y), its samples averaged, as
/// B, G, R, A bytes.
fn pixel(scene: &dyn Scene, x: f32, y: f32, samples: u16) -> [u8; 4] {
    let step = 1.0 / f32::from(samples);
    let mut sum = [0.0f32; 4];
    for j in 0..samples {
        for i in 0..samples {
            let point = Point {
                x: x + (f32::from(i) + 0.5) * step,
                y: y + (f32::from(j) + 0.5) * step,
            };
            let colour = scene.colour(point);
            sum.iter_mut()
                .zip(colour)
                .for_each(|(total, part)| *total += part);
        }
    }
    let all = f32::from(samples * samples);
    let byte = |value: f32| (value / all * 255.0).round().clamp(0.0, 255.0) as u8;
    [byte(sum[2]), byte(sum[1]), byte(sum[0]), byte(sum[3])]
}

/// Premultiplies a straight RGBA colour.
fn premultiplied([r, g, b, a]: [f32; 4]) -> [f32; 4] {
    [r * a, g * a, b * a, a]
}

/// Whether `point` lies on a rounded rectangle filling `size`.
fn on_tile(point: Point, size: (f32, f32), radius: f32) -> bool {
    let dx = (radius - point.x).max(point.x - (size.0 - radius)).max(0.0);
    let dy = (radius - point.y).max(point.y - (size.1 - radius)).max(0.0);
    dx * dx + dy * dy <= radius * radius
}

/// The countdown: the digits in the job's colour with a thin white
/// outline; every other pixel fully transparent.
struct Number {
    lit: Vec<Rect>,
    outline: Vec<Rect>,
}

impl Number {
    fn new(seconds: u64, (width, height): (u32, u32)) -> Number {
        let size = (width as f32, height as f32);
        let text = glyphs::label(seconds);
        let characters = (size.1 / (1.0 + 2.0 * PAD)).floor();
        let origin = Point {
            x: (size.0 - characters * glyphs::span(&text)) / 2.0,
            y: (size.1 - characters) / 2.0,
        };
        let lit = glyphs::layout(&text, origin, characters);
        let edge = (characters * OUTLINE).max(1.0).round();
        let outline = lit
            .iter()
            .map(|rect| Rect {
                x: rect.x - edge,
                y: rect.y - edge,
                right: rect.right + edge,
                bottom: rect.bottom + edge,
            })
            .collect();
        Number { lit, outline }
    }
}

impl Scene for Number {
    fn colour(&self, point: Point) -> [f32; 4] {
        if self.lit.iter().any(|rect| rect.contains(point)) {
            DIGITS
        } else if self.outline.iter().any(|rect| rect.contains(point)) {
            WHITE
        } else {
            [0.0; 4]
        }
    }
}

struct Badge {
    side: f32,
}

impl Badge {
    fn new((width, _): (u32, u32)) -> Badge {
        Badge { side: width as f32 }
    }
}

impl Scene for Badge {
    fn colour(&self, point: Point) -> [f32; 4] {
        let side = self.side;
        let (dx, dy) = (point.x - side / 2.0, point.y - side / 2.0);
        if !on_tile(point, (side, side), side * 0.22) {
            [0.0; 4]
        } else if dx * dx + dy * dy <= (side * 0.25) * (side * 0.25) {
            WHITE
        } else {
            premultiplied(TILE)
        }
    }
}

/// Opaque one-pixel stripes: columns on the left half, rows on the right.
fn stripes((width, height): (u32, u32)) -> Image {
    let mut bytes = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let parity = if x < width / 2 { x } else { y };
            let value = if parity % 2 == 0 { 0 } else { 255 };
            bytes.extend([value, value, value, 255]);
        }
    }
    Image {
        width,
        height,
        bytes,
    }
}
