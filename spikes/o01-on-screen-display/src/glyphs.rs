//! The countdown's characters as rectangles: the tray icon's seven segments
//! (crates/auto-clicker/src/tray_icon.rs), laid out by height instead of by
//! an icon's side.

/// A digit's width, as a share of its height.
const DIGIT: f32 = 0.55;
/// The "m"'s width, as a share of the height.
const MINUTES: f32 = 0.7;
/// The space between characters, as a share of their height.
pub const GAP: f32 = 0.12;
/// A segment's thickness, as a share of the character's height.
const THICKNESS: f32 = 0.17;

/// A point in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// A rectangle in pixels, from (x, y) to (right, bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            right: x + width,
            bottom: y + height,
        }
    }

    pub fn contains(&self, point: Point) -> bool {
        (self.x..self.right).contains(&point.x) && (self.y..self.bottom).contains(&point.y)
    }
}

#[derive(Clone, Copy, Debug)]
enum Segment {
    Top,
    UpperRight,
    LowerRight,
    Bottom,
    LowerLeft,
    UpperLeft,
    Middle,
}

use Segment::*;

const DIGITS: [&[Segment]; 10] = [
    &[Top, UpperRight, LowerRight, Bottom, LowerLeft, UpperLeft],
    &[UpperRight, LowerRight],
    &[Top, UpperRight, Middle, LowerLeft, Bottom],
    &[Top, UpperRight, Middle, LowerRight, Bottom],
    &[UpperLeft, Middle, UpperRight, LowerRight],
    &[Top, UpperLeft, Middle, LowerRight, Bottom],
    &[Top, UpperLeft, Middle, LowerRight, Bottom, LowerLeft],
    &[Top, UpperRight, LowerRight],
    &[
        Top, UpperRight, LowerRight, Bottom, LowerLeft, UpperLeft, Middle,
    ],
    &[Top, UpperRight, LowerRight, Bottom, UpperLeft, Middle],
];

/// A character's box and stroke, in whole pixels.
#[derive(Clone, Copy, Debug)]
struct Cell {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    thickness: f32,
}

impl Cell {
    fn middle(&self) -> f32 {
        self.y + ((self.height - self.thickness) / 2.0).floor()
    }

    fn segment(&self, segment: Segment) -> Rect {
        let Cell {
            x,
            y,
            width,
            height,
            thickness,
        } = *self;
        let middle = self.middle();
        let upper = middle - y + thickness;
        let lower = y + height - middle;
        let right = x + width - thickness;
        match segment {
            Top => Rect::new(x, y, width, thickness),
            Middle => Rect::new(x, middle, width, thickness),
            Bottom => Rect::new(x, y + height - thickness, width, thickness),
            UpperLeft => Rect::new(x, y, thickness, upper),
            UpperRight => Rect::new(right, y, thickness, upper),
            LowerLeft => Rect::new(x, middle, thickness, lower),
            LowerRight => Rect::new(right, middle, thickness, lower),
        }
    }

    fn minutes(&self) -> Vec<Rect> {
        let stroke = (self.thickness * 0.7).round();
        let top = self.middle();
        let leg = self.y + self.height - top;
        let centre = self.x + ((self.width - stroke) / 2.0).round();
        let right = self.x + self.width - stroke;
        [self.x, centre, right]
            .into_iter()
            .map(|x| Rect::new(x, top, stroke, leg))
            .chain([Rect::new(self.x, top, self.width, stroke)])
            .collect()
    }

    fn character(&self, character: char) -> Vec<Rect> {
        if character == 'm' {
            return self.minutes();
        }
        let lit = character
            .to_digit(10)
            .and_then(|digit| DIGITS.get(usize::try_from(digit).ok()?));
        lit.into_iter()
            .flat_map(|segments| segments.iter())
            .map(|&lit| self.segment(lit))
            .collect()
    }
}

fn advance(character: char) -> f32 {
    if character == 'm' { MINUTES } else { DIGIT }
}

/// The label for `seconds` left, as the tray icon shows it.
pub fn label(seconds: u64) -> String {
    if seconds <= 99 {
        seconds.to_string()
    } else {
        format!("{}m", seconds.div_ceil(60).min(99))
    }
}

/// The label's width, as a share of its height.
pub fn span(text: &str) -> f32 {
    let gaps = text.chars().count().saturating_sub(1) as f32;
    text.chars().map(advance).sum::<f32>() + gaps * GAP
}

/// The rectangles that draw `text` with characters `height` pixels tall,
/// its top left corner at `origin`, on whole pixels.
pub fn layout(text: &str, origin: Point, height: f32) -> Vec<Rect> {
    let height = height.floor();
    let pixels = |share: f32| (share * height).round();
    let (gap, thickness) = (pixels(GAP), pixels(THICKNESS));
    let mut x = origin.x.floor();
    let mut rects = Vec::new();
    for character in text.chars() {
        let width = pixels(advance(character));
        let cell = Cell {
            x,
            y: origin.y.floor(),
            width,
            height,
            thickness,
        };
        rects.extend(cell.character(character));
        x += width + gap;
    }
    rects
}
