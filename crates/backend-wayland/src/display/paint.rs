//! What the display draws into a buffer.
//!
//! - **The countdown:** the digits in the job's colour with a thin white
//!   outline, and every other pixel fully transparent. The digits and the
//!   outline are rectangles on whole pixels, so each is filled row by row,
//!   with no sampling (decision 6 of the brief).
//! - **The badge:** a tile with rounded corners in the job's colour and a
//!   white mark, each pixel sampled 4 by 4 times, as the tray icon's tile
//!   is: at its size, a fraction of a millisecond (0005).

use auto_clicker_core::badge::{self, Badge};
use auto_clicker_core::digits::{Label, Point, Rect, Rgb, Size};
use auto_clicker_core::tile::{self, Tile};

use super::fit::Fit;

/// The outline, as a share of the characters' height.
const OUTLINE: f32 = 0.025;
/// The outline's colour, premultiplied ARGB8888 in memory order.
const WHITE: [u8; 4] = [0xff; 4];

/// Pixels in ARGB8888, premultiplied, little endian: blue, green, red,
/// alpha in memory, `width * 4` bytes a row.
pub(crate) struct Picture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bytes: Vec<u8>,
}

/// `label` in `colour` with its outline, centred in a buffer of `size`.
pub(crate) fn number(label: &Label, colour: Rgb, size: Fit) -> Picture {
    let (lit, outline) = shapes(label, size);
    let mut picture = Picture::clear(size.width, size.height);
    let Rgb(r, g, b) = colour;
    for rect in &outline {
        picture.fill(rect, WHITE);
    }
    for rect in &lit {
        picture.fill(rect, [b, g, r, 0xff]);
    }
    picture
}

/// `badge` filling a buffer of `size`, square: its tile, and the mark on it.
pub(crate) fn badge(badge: Badge, size: Fit) -> Picture {
    let mut picture = Picture::clear(size.width, size.height);
    // At most BADGE_MOST: exact as floats.
    let side = size.width.min(size.height) as f32;
    let shape = Tile::new(side);
    let colour = |point: Point| {
        let unit = Point {
            x: point.x / side,
            y: point.y / side,
        };
        if !shape.contains(point) {
            None
        } else if badge.mark.contains(unit) {
            Some(badge::MARK)
        } else {
            Some(badge.tile)
        }
    };
    let stride = size.width as usize * 4;
    for y in 0..size.height {
        for x in 0..size.width {
            let corner = Point {
                x: x as f32,
                y: y as f32,
            };
            let start = y as usize * stride + x as usize * 4;
            let pixel = tile::sample(corner, colour).premultiplied_bgra();
            picture.bytes[start..start + 4].copy_from_slice(&pixel);
        }
    }
    picture
}

/// The digits' rectangles and their outline's, centred in `size`.
pub(crate) fn shapes(label: &Label, size: Fit) -> (Vec<Rect>, Vec<Rect>) {
    // At most MAX_SIDE: exact as floats.
    let picture = Size {
        width: size.width as f32,
        height: size.height as f32,
    };
    let characters = size.characters as f32;
    let lit = label.centred(picture, characters);
    let edge = (characters * OUTLINE).round().max(1.0);
    let outline = lit.iter().map(|rect| rect.grown(edge)).collect();
    (lit, outline)
}

impl Picture {
    /// A fully transparent picture: the length is at most 64 MiB, since
    /// each side is at most `MAX_SIDE`.
    fn clear(width: u32, height: u32) -> Picture {
        let length = (width as usize) * (height as usize) * 4;
        Picture {
            width,
            height,
            bytes: vec![0; length],
        }
    }

    /// Fills the pixels `rect` covers, clipped to the picture, with `pixel`.
    fn fill(&mut self, rect: &Rect, pixel: [u8; 4]) {
        let clip = |from: f32, limit: u32| from.clamp(0.0, limit as f32) as usize;
        let (left, right) = (clip(rect.x, self.width), clip(rect.right, self.width));
        let (top, bottom) = (clip(rect.y, self.height), clip(rect.bottom, self.height));
        let stride = self.width as usize * 4;
        for row in top..bottom {
            let start = row * stride;
            let span = &mut self.bytes[start + left * 4..start + right * 4];
            for each in span.as_chunks_mut::<4>().0 {
                *each = pixel;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use auto_clicker_core::badge::Mark;
    use auto_clicker_core::config::Corner;
    use auto_clicker_core::digits::{CLICKING, RECORDING};

    use super::*;
    use crate::display::fit::Place;

    fn size(width: u32, height: u32, characters: u32) -> Fit {
        Fit {
            width,
            height,
            characters,
            place: Place::Middle,
        }
    }

    /// The pixel at (x, y).
    fn at(picture: &Picture, x: u32, y: u32) -> [u8; 4] {
        let index = ((y * picture.width + x) * 4) as usize;
        picture.bytes[index..index + 4].try_into().unwrap()
    }

    #[test]
    fn the_buffer_holds_the_digits_and_their_outline_and_nothing_else() {
        for (seconds, colour, buffer) in [
            (5, CLICKING, size(378, 540, 360)),
            (5941, RECORDING, size(914, 540, 360)),
            (88, CLICKING, size(316, 450, 300)),
            (1, RECORDING, size(20, 30, 20)),
        ] {
            let label = Label::of(seconds);
            let picture = number(&label, colour, buffer);
            let (lit, outline) = shapes(&label, buffer);
            let Rgb(r, g, b) = colour;
            let mut drawn = 0;
            for y in 0..buffer.height {
                for x in 0..buffer.width {
                    let point = Point {
                        x: x as f32 + 0.5,
                        y: y as f32 + 0.5,
                    };
                    let expected = if lit.iter().any(|rect| rect.contains(point)) {
                        [b, g, r, 0xff]
                    } else if outline.iter().any(|rect| rect.contains(point)) {
                        WHITE
                    } else {
                        [0; 4]
                    };
                    drawn += usize::from(expected != [0; 4]);
                    assert_eq!(at(&picture, x, y), expected, "{seconds} at ({x}, {y})");
                }
            }
            assert!(drawn > 0, "{seconds}");
        }
    }

    #[test]
    fn the_outline_is_thin_and_the_digits_are_centred_inside_the_padding() {
        let buffer = size(914, 540, 360);
        let (lit, outline) = shapes(&Label::of(5941), buffer);
        // 2.5 % of 360, rounded.
        assert_eq!(outline[0].x, lit[0].x - 9.0);
        assert_eq!(outline[0].bottom, lit[0].bottom + 9.0);
        let left = outline.iter().map(|r| r.x).fold(f32::MAX, f32::min);
        let right = outline.iter().map(|r| r.right).fold(f32::MIN, f32::max);
        let top = outline.iter().map(|r| r.y).fold(f32::MAX, f32::min);
        let bottom = outline.iter().map(|r| r.bottom).fold(f32::MIN, f32::max);
        assert!(left > 0.0 && top > 0.0, "{left}, {top}");
        assert!(right < 914.0 && bottom < 540.0, "{right}, {bottom}");
        assert!((left - (914.0 - right)).abs() <= 1.0);
        assert!((top - (540.0 - bottom)).abs() <= 1.0);
        // The thinnest outline is a pixel.
        let (lit, outline) = shapes(&Label::of(1), size(20, 30, 20));
        assert_eq!(outline[0].x, lit[0].x - 1.0);
        // A "1" is centred by its strokes, not by its cell.
        let (lit, _) = shapes(&Label::of(1), size(256, 405, 270));
        let left = lit.iter().map(|r| r.x).fold(f32::MAX, f32::min);
        let right = lit.iter().map(|r| r.right).fold(f32::MIN, f32::max);
        assert!((left - (256.0 - right)).abs() <= 1.0, "{left}, {right}");
    }

    #[test]
    fn a_shape_past_the_edge_is_clipped() {
        let mut picture = Picture::clear(4, 3);
        let corner = Point { x: -2.0, y: 1.0 };
        let size = Size {
            width: 10.0,
            height: 5.0,
        };
        picture.fill(&Rect::new(corner, size), WHITE);
        for y in 0..3 {
            for x in 0..4 {
                let expected = if y >= 1 { WHITE } else { [0; 4] };
                assert_eq!(at(&picture, x, y), expected, "({x}, {y})");
            }
        }
    }

    #[test]
    fn the_badge_is_its_tile_with_the_mark_and_clear_round_the_corners() {
        let clicking = Badge {
            mark: Mark::Pointer,
            tile: CLICKING,
        };
        let recording = Badge {
            mark: Mark::Dot,
            tile: RECORDING,
        };
        let playing = Badge {
            mark: Mark::Triangle,
            tile: CLICKING,
        };
        for side in [40, 50, 64] {
            let fit = Fit::badge(Corner::TopLeft, (1920, 1080)).unwrap();
            let fit = Fit {
                width: side,
                height: side,
                ..fit
            };
            for badge in [clicking, recording, playing] {
                let picture = super::badge(badge, fit);
                assert_eq!(picture.bytes.len(), (side * side * 4) as usize);
                let Rgb(r, g, b) = badge.tile;
                let last = side - 1;
                // The corners are clear, the edges' middles the tile's
                // colour, the middle the mark's white.
                for (x, y) in [(0, 0), (last, 0), (0, last), (last, last)] {
                    assert_eq!(at(&picture, x, y), [0; 4], "{badge:?} at {side}");
                }
                for (x, y) in [(1, side / 2), (side / 2, 1), (last - 1, side / 2)] {
                    assert_eq!(at(&picture, x, y), [b, g, r, 0xff], "{badge:?} at {side}");
                }
                let middle = if badge.mark == Mark::Pointer {
                    (side * 2 / 5, side * 9 / 20)
                } else {
                    (side / 2, side / 2)
                };
                assert_eq!(
                    at(&picture, middle.0, middle.1),
                    [0xff; 4],
                    "{badge:?} at {side}"
                );
                // Some pixels on the rounded corners are partly covered,
                // premultiplied: no channel past the alpha.
                let partial = picture
                    .bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[3] > 0 && p[3] < 0xff)
                    .inspect(|p| assert!(p[..3].iter().all(|&c| c <= p[3]), "{p:?}"))
                    .count();
                assert!(partial >= 8, "{badge:?} at {side}: {partial}");
            }
        }
    }
}
