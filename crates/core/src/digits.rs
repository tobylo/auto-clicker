//! A countdown's digits, as rectangles: what the tray icon and the
//! on-screen display both draw. A label is the seconds left up to 99, then
//! whole minutes, such as "5m"; each character is a box of seven segments,
//! or an "m" of a bar and three legs, laid out on whole pixels so that the
//! strokes stay sharp at any size.
//!
//! Nothing here draws a pixel: the tray icon samples the rectangles over
//! its tile, the display fills them.

use crate::controller::Activity;

/// A countdown's largest label: past 99 minutes it stays at "99m".
const MOST: u64 = 99;

/// A digit's width, as a share of its height.
const DIGIT: f32 = 0.55;
/// The "m"'s width, as a share of the height: wider, for its three legs.
const MINUTES: f32 = 0.7;
/// The space between characters, as a share of their height.
const GAP: f32 = 0.12;
/// A segment's thickness, as a share of the character's height.
const THICKNESS: f32 = 0.17;

/// A colour, red, green and blue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// Clicking and playback: the clicking icon's indigo.
pub const CLICKING: Rgb = Rgb(0x43, 0x38, 0xca);
/// Recording: the recording icon's red.
pub const RECORDING: Rgb = Rgb(0xef, 0x44, 0x44);

/// The seconds left and the job's colour while `activity` counts down
/// from the tray; `None` otherwise.
pub fn countdown(activity: Activity) -> Option<(u64, Rgb)> {
    match activity {
        Activity::StartingIn(seconds) | Activity::PlayingIn(seconds) => Some((seconds, CLICKING)),
        Activity::RecordingIn(seconds) => Some((seconds, RECORDING)),
        _ => None,
    }
}

/// What a countdown shows: two characters at most, three for "10m" and
/// over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label(String);

impl Label {
    /// The label for `seconds` left: the seconds up to 99, then whole
    /// minutes, rounded up, such as "5m".
    pub fn of(seconds: u64) -> Label {
        Label(if seconds <= MOST {
            seconds.to_string()
        } else {
            format!("{}m", seconds.div_ceil(60).min(MOST))
        })
    }

    /// The characters shown.
    pub fn text(&self) -> &str {
        &self.0
    }

    /// The label's width as a share of its height, before rounding.
    pub fn span(&self) -> f32 {
        let advances: f32 = self.characters().map(Character::advance).sum();
        advances + self.gaps() * GAP
    }

    /// The label's width in whole pixels at characters `height` pixels
    /// high, as [`Label::draw`] lays it out.
    pub fn width(&self, height: f32) -> f32 {
        let pixels = |share: f32| (share * height).round();
        let widths: f32 = self.characters().map(|c| pixels(c.advance())).sum();
        widths + self.gaps() * pixels(GAP)
    }

    /// The rectangles that draw the label with its top left corner at
    /// `corner` and characters `height` pixels high. With `corner` and
    /// `height` on whole pixels, so is every edge.
    pub fn draw(&self, corner: Point, height: f32) -> Vec<Rect> {
        let pixels = |share: f32| (share * height).round();
        let (gap, thickness) = (pixels(GAP), pixels(THICKNESS));
        let mut x = corner.x;
        let mut rects = Vec::new();
        for character in self.characters() {
            let width = pixels(character.advance());
            let cell = Cell {
                x,
                y: corner.y,
                width,
                height,
                thickness,
            };
            rects.extend(cell.character(character));
            x += width + gap;
        }
        rects
    }

    /// The label's characters, as their shapes.
    fn characters(&self) -> impl Iterator<Item = Character> + '_ {
        self.0.chars().map(|character| match character {
            'm' => Character::Minutes,
            _ => character
                .to_digit(10)
                .and_then(|digit| DIGITS.get(usize::try_from(digit).ok()?))
                .map_or(Character::Unknown, |lit| Character::Digit(lit)),
        })
    }

    /// How many gaps lie between the characters.
    fn gaps(&self) -> f32 {
        // A label has three characters at most: exact as a float.
        self.0.chars().count().saturating_sub(1) as f32
    }
}

/// A character as it is drawn.
#[derive(Clone, Copy, Debug)]
enum Character {
    /// A digit, by its lit segments.
    Digit(&'static [Segment]),
    /// The "m" of minutes.
    Minutes,
    /// A character there is no shape for: as wide as a digit, and blank.
    Unknown,
}

impl Character {
    /// Its width, as a share of its height.
    fn advance(self) -> f32 {
        match self {
            Character::Minutes => MINUTES,
            Character::Digit(_) | Character::Unknown => DIGIT,
        }
    }
}

/// A point in pixels, from the picture's top left corner.
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

/// A width and a height in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// The rectangle at `corner`, `size` large.
    pub fn new(corner: Point, size: Size) -> Rect {
        Rect {
            x: corner.x,
            y: corner.y,
            right: corner.x + size.width,
            bottom: corner.y + size.height,
        }
    }

    pub fn contains(&self, point: Point) -> bool {
        (self.x..self.right).contains(&point.x) && (self.y..self.bottom).contains(&point.y)
    }

    /// The same, `by` pixels larger on every side.
    pub fn grown(&self, by: f32) -> Rect {
        Rect {
            x: self.x - by,
            y: self.y - by,
            right: self.right + by,
            bottom: self.bottom + by,
        }
    }
}

/// The seven segments of a digit.
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

/// The lit segments of each digit, 0 to 9.
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
    /// The middle bar's top edge, on a whole pixel.
    fn middle(&self) -> f32 {
        self.y + ((self.height - self.thickness) / 2.0).floor()
    }

    /// Where `segment` lies in this cell.
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
        let rect = |x, y, width, height| Rect::new(Point { x, y }, Size { width, height });
        match segment {
            Top => rect(x, y, width, thickness),
            Middle => rect(x, middle, width, thickness),
            Bottom => rect(x, y + height - thickness, width, thickness),
            UpperLeft => rect(x, y, thickness, upper),
            UpperRight => rect(right, y, thickness, upper),
            LowerLeft => rect(x, middle, thickness, lower),
            LowerRight => rect(right, middle, thickness, lower),
        }
    }

    /// A lowercase "m" in the lower half: a bar and three legs, thinner
    /// than a digit's strokes so that the gaps between them show.
    fn minutes(&self) -> Vec<Rect> {
        let stroke = (self.thickness * 0.7).round();
        let top = self.middle();
        let leg = self.y + self.height - top;
        let centre = self.x + ((self.width - stroke) / 2.0).round();
        let right = self.x + self.width - stroke;
        let rect = |x, width, height| Rect::new(Point { x, y: top }, Size { width, height });
        [self.x, centre, right]
            .into_iter()
            .map(|x| rect(x, stroke, leg))
            .chain([rect(self.x, self.width, stroke)])
            .collect()
    }

    /// The rectangles that draw `character`; none for one there is no
    /// shape for.
    fn character(&self, character: Character) -> Vec<Rect> {
        match character {
            Character::Minutes => self.minutes(),
            Character::Digit(segments) => segments.iter().map(|&lit| self.segment(lit)).collect(),
            Character::Unknown => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
