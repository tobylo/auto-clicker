//! The state's pictures as `data/icons/hicolor/scalable/apps` draws them,
//! drawn in code for Windows, which wants pixels rather than a themed icon:
//! a rounded tile with a vertical gradient, a mouse with its left button lit
//! and a lightning bolt, and for recording a red dot. The shapes are the
//! SVGs', on their 64-unit grid; a change to one is made in both.

use auto_clicker_core::digits::{Point, Rgb};
use auto_clicker_core::tile::{self, Tile};

use super::{Image, Picture, WHITE};

/// The SVGs' grid: 64 units a side.
const GRID: f32 = 64.0;

/// The mouse: a capsule, as the SVGs' `rect x="18" y="8" width="28"
/// height="48" rx="14"`.
const MOUSE: Capsule = Capsule {
    left: 18.0,
    top: 8.0,
    right: 46.0,
    bottom: 56.0,
    radius: 14.0,
};
/// Where the mouse's buttons end, and where they part.
const BUTTONS_END: f32 = 26.0;
const BUTTONS_PART: f32 = 32.0;
/// The lines between the buttons and below them: 2 units wide.
const LINE: f32 = 1.0;
/// The lightning bolt: `M35 30l-9 13h6l-3 10 10-14h-6z`.
const BOLT: [Point; 6] = [
    Point { x: 35.0, y: 30.0 },
    Point { x: 26.0, y: 43.0 },
    Point { x: 32.0, y: 43.0 },
    Point { x: 29.0, y: 53.0 },
    Point { x: 39.0, y: 39.0 },
    Point { x: 33.0, y: 39.0 },
];
/// The recording dot: `circle cx="50" cy="50" r="11"`, with a white ring
/// 3 units wide around its edge.
const DOT: Point = Point { x: 50.0, y: 50.0 };
const DOT_RADIUS: f32 = 11.0;
const RING: f32 = 1.5;
const RED: Rgb = Rgb(0xef, 0x44, 0x44);

/// A picture's colours: the tile's gradient from top to bottom, which the
/// marks share, and the lit button's.
struct Colours {
    top: Rgb,
    bottom: Rgb,
    button: Rgb,
}

const IDLE: Colours = Colours {
    top: Rgb(0x64, 0x74, 0x8b),
    bottom: Rgb(0x47, 0x55, 0x69),
    button: Rgb(0xcb, 0xd5, 0xe1),
};
const CLICKING: Colours = Colours {
    top: Rgb(0x63, 0x66, 0xf1),
    bottom: Rgb(0x43, 0x38, 0xca),
    button: Rgb(0xfa, 0xcc, 0x15),
};

/// `picture`, `side` pixels square.
pub fn draw(picture: Picture, side: u16) -> Image {
    let colours = match picture {
        Picture::Clicking => &CLICKING,
        Picture::Idle | Picture::Recording => &IDLE,
    };
    let dot = picture == Picture::Recording;
    let shape = Tile::new(GRID);
    let scale = GRID / f32::from(side);
    Image::sampled(side, |corner| {
        tile::sample(corner, |point| {
            let unit = Point {
                x: point.x * scale,
                y: point.y * scale,
            };
            colour(unit, &shape, colours, dot)
        })
    })
}

/// The colour at `point`, in grid units: the topmost shape there, or
/// `None` off the tile.
fn colour(point: Point, shape: &Tile, colours: &Colours, dot: bool) -> Option<Rgb> {
    if dot && let Some(colour) = recording_dot(point) {
        return Some(colour);
    }
    if !shape.contains(point) {
        return None;
    }
    Some(mouse(point, colours).unwrap_or_else(|| gradient(colours, point.y)))
}

/// The mouse's colour at `point`; none off it.
fn mouse(point: Point, colours: &Colours) -> Option<Rgb> {
    if on_lines(point) || inside(point, &BOLT) {
        return Some(colours.bottom);
    }
    if !MOUSE.contains(point) {
        return None;
    }
    let lit = point.x < BUTTONS_PART && point.y < BUTTONS_END;
    Some(if lit { colours.button } else { WHITE })
}

/// Whether `point` is on the line between the buttons or the one below
/// them.
fn on_lines(Point { x, y }: Point) -> bool {
    let between = (x - BUTTONS_PART).abs() <= LINE && (MOUSE.top..=BUTTONS_END).contains(&y);
    let below = (y - BUTTONS_END).abs() <= LINE && (MOUSE.left..=MOUSE.right).contains(&x);
    between || below
}

/// The recording dot's colour at `point`: its ring, its fill, or none off it.
fn recording_dot(point: Point) -> Option<Rgb> {
    let distance = (point.x - DOT.x).hypot(point.y - DOT.y);
    if distance <= DOT_RADIUS - RING {
        Some(RED)
    } else if distance <= DOT_RADIUS + RING {
        Some(WHITE)
    } else {
        None
    }
}

/// The tile's colour `y` units down.
fn gradient(colours: &Colours, y: f32) -> Rgb {
    let share = (y / GRID).clamp(0.0, 1.0);
    let mix = |top: u8, bottom: u8| {
        let value = f32::from(top) + (f32::from(bottom) - f32::from(top)) * share;
        // Between two bytes: always a byte.
        value.round() as u8
    };
    let (Rgb(r0, g0, b0), Rgb(r1, g1, b1)) = (colours.top, colours.bottom);
    Rgb(mix(r0, r1), mix(g0, g1), mix(b0, b1))
}

/// A rectangle whose corners are rounded by `radius`.
struct Capsule {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
}

impl Capsule {
    fn contains(&self, Point { x, y }: Point) -> bool {
        let dx = (self.left + self.radius - x)
            .max(x - (self.right - self.radius))
            .max(0.0);
        let dy = (self.top + self.radius - y)
            .max(y - (self.bottom - self.radius))
            .max(0.0);
        (self.left..=self.right).contains(&x)
            && (self.top..=self.bottom).contains(&y)
            && dx * dx + dy * dy <= self.radius * self.radius
    }
}

/// Whether `point` lies inside `polygon` (even-odd rule).
fn inside(point: Point, polygon: &[Point]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for (index, a) in polygon.iter().enumerate() {
        let b = polygon[previous];
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
        previous = index;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::super::tests::{at, opaque};
    use super::*;

    /// Each picture is checked on the SVGs' grid: 64 pixels square, one
    /// pixel per unit.
    #[test]
    fn each_picture_draws_the_svgs_shapes_in_its_colours() {
        let idle = draw(Picture::Idle, 64);
        let clicking = draw(Picture::Clicking, 64);
        let recording = draw(Picture::Recording, 64);
        for image in [&idle, &clicking, &recording] {
            assert_eq!(image.rgba.len(), 64 * 64 * 4);
            // The corners are clear; the mouse's right button is white.
            assert_eq!(at(image, 0, 0)[3], 0);
            assert_eq!(at(image, 40, 20), opaque(WHITE));
            // The bolt and the line between the buttons, in the tile's
            // darker colour.
            assert_eq!(at(image, 32, 40)[..3], at(image, 31, 14)[..3]);
        }
        // The lit left button.
        assert_eq!(at(&idle, 25, 18), opaque(IDLE.button));
        assert_eq!(at(&clicking, 25, 18), opaque(CLICKING.button));
        // The tile's gradient: lighter at the top.
        assert!(at(&idle, 5, 2)[0] > at(&idle, 5, 60)[0]);
        // Only recording has the dot, red inside a white ring.
        assert_eq!(at(&recording, 50, 50), opaque(RED));
        assert_eq!(at(&recording, 61, 50), opaque(WHITE));
        assert_ne!(at(&idle, 50, 50), opaque(RED));
    }

    #[test]
    fn a_picture_is_drawn_at_any_size() {
        for side in [16, 20, 24, 32] {
            let image = draw(Picture::Clicking, side);
            assert_eq!(image.side, side);
            assert_eq!(image.rgba.len(), usize::from(side).pow(2) * 4);
            assert_eq!(at(&image, 0, 0)[3], 0, "{side} px");
            let middle = i32::from(side) / 2;
            assert_eq!(at(&image, 1, middle)[3], 0xff, "{side} px");
        }
    }

    #[test]
    fn the_bolt_is_inside_its_outline_only() {
        assert!(inside(Point { x: 32.0, y: 40.0 }, &BOLT));
        assert!(!inside(Point { x: 26.0, y: 32.0 }, &BOLT));
        assert!(!inside(Point { x: 38.0, y: 50.0 }, &BOLT));
    }
}
