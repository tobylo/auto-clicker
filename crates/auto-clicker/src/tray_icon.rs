//! The tray icon during a countdown: the time left, drawn in code over a tile
//! in the job's colour. Plasma shows these pixmaps while the icon has no name.
//!
//! Nothing here keeps state or wakes on its own: the pixmaps are drawn from
//! the view each time it changes, which a countdown does once a second.

use auto_clicker_core::controller::Activity;

/// The sizes Plasma picks from: the default panel's, and larger panels' or
/// scaled screens'.
const SIZES: [u16; 3] = [22, 32, 48];

/// Samples per pixel along each axis, for smooth edges.
const SAMPLES: u16 = 4;

/// A countdown's largest label: past 99 minutes it stays at "99m".
const MOST: u64 = 99;

/// Height of a character, as a share of the icon's side, when it fits.
const HEIGHT: f32 = 0.74;
/// The widest the label may be, as a share of the icon's side.
const WIDTH: f32 = 0.88;
/// A digit's width, as a share of its height.
const DIGIT: f32 = 0.55;
/// The "m"'s width, as a share of the height: wider, for its three legs.
const MINUTES: f32 = 0.7;
/// The space between characters, as a share of their height.
const GAP: f32 = 0.12;
/// A segment's thickness, as a share of the character's height.
const THICKNESS: f32 = 0.17;
/// The tile's corner radius, as a share of its side, as in the themed icons.
const RADIUS: f32 = 14.0 / 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

/// The digits' colour.
const WHITE: Rgb = Rgb(0xff, 0xff, 0xff);
/// Clicking and playback: the clicking icon's indigo.
const CLICKING: Rgb = Rgb(0x43, 0x38, 0xca);
/// Recording: the recording icon's red.
const RECORDING: Rgb = Rgb(0xef, 0x44, 0x44);

/// Whether `activity` is a countdown, which shows on the icon.
pub fn counts_down(activity: Activity) -> bool {
    countdown(activity).is_some()
}

/// The icon's pixmaps for `activity`: one per size during a countdown, none
/// otherwise.
pub fn pixmaps(activity: Activity) -> Vec<ksni::Icon> {
    let Some((seconds, tile)) = countdown(activity) else {
        return Vec::new();
    };
    let label = Label::of(seconds);
    SIZES
        .iter()
        .map(|&size| render(&label, tile, size))
        .collect()
}

/// The seconds left and the tile's colour of a countdown.
fn countdown(activity: Activity) -> Option<(u64, Rgb)> {
    match activity {
        Activity::StartingIn(seconds) | Activity::PlayingIn(seconds) => Some((seconds, CLICKING)),
        Activity::RecordingIn(seconds) => Some((seconds, RECORDING)),
        _ => None,
    }
}

/// What the icon shows: two characters at most, three for "10m" and over.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Label(String);

impl Label {
    /// The label for `seconds` left: the seconds up to 99, then whole
    /// minutes, rounded up, such as "5m".
    fn of(seconds: u64) -> Label {
        Label(if seconds <= MOST {
            seconds.to_string()
        } else {
            format!("{}m", seconds.div_ceil(60).min(MOST))
        })
    }

    /// The rectangles that draw the label centred on an icon `side` pixels
    /// wide, as large as fits and on whole pixels, so that the strokes stay
    /// sharp. A single digit is no smaller than each of two.
    fn layout(&self, side: f32) -> Vec<Rect> {
        let text = &self.0;
        let gaps = text.chars().count().saturating_sub(1) as f32;
        let span = text.chars().map(advance).sum::<f32>() + gaps * GAP;
        let height = (HEIGHT * side).min(WIDTH * side / span).floor();
        let pixels = |share: f32| (share * height).round().max(1.0);
        let (gap, thickness) = (pixels(GAP), pixels(THICKNESS));
        let widths: Vec<f32> = text.chars().map(|c| pixels(advance(c))).collect();
        let total = widths.iter().sum::<f32>() + gaps * gap;
        let mut x = ((side - total) / 2.0).floor();
        let y = ((side - height) / 2.0).floor();
        let mut rects = Vec::new();
        for (character, width) in text.chars().zip(widths) {
            let cell = Cell {
                x,
                y,
                width,
                height,
                thickness,
            };
            rects.extend(cell.character(character));
            x += width + gap;
        }
        rects
    }
}

/// A point in pixels, from the icon's top left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    x: f32,
    y: f32,
}

/// A rectangle in pixels, from (x, y) to (right, bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f32,
    y: f32,
    right: f32,
    bottom: f32,
}

impl Rect {
    fn new(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            right: x + width,
            bottom: y + height,
        }
    }

    fn contains(&self, point: Point) -> bool {
        (self.x..self.right).contains(&point.x) && (self.y..self.bottom).contains(&point.y)
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

    /// A lowercase "m" in the lower half: a bar and three legs, thinner
    /// than a digit's strokes so that the gaps between them show.
    fn minutes(&self) -> Vec<Rect> {
        let stroke = (self.thickness * 0.7).round().max(1.0);
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

    /// The rectangles that draw `character`; none for a character the icon
    /// has no shape for.
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

/// A character's width, as a share of its height.
fn advance(character: char) -> f32 {
    if character == 'm' { MINUTES } else { DIGIT }
}

/// What an icon shows: the label's strokes on a tile with rounded corners.
struct Scene {
    side: f32,
    radius: f32,
    lit: Vec<Rect>,
    tile: Rgb,
}

impl Scene {
    fn new(label: &Label, tile: Rgb, side: f32) -> Scene {
        Scene {
            side,
            radius: RADIUS * side,
            lit: label.layout(side),
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
    let scene = Scene::new(label, tile, f32::from(size));
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
    use super::*;

    /// The ARGB pixel at (x, y) of `icon`.
    fn at(icon: &ksni::Icon, x: i32, y: i32) -> [u8; 4] {
        let index = usize::try_from((y * icon.width + x) * 4).unwrap();
        icon.data[index..index + 4].try_into().unwrap()
    }

    fn text(text: &str) -> Label {
        Label(text.to_owned())
    }

    fn opaque(Rgb(r, g, b): Rgb) -> [u8; 4] {
        [0xff, r, g, b]
    }

    /// The centre of `rect`, in whole pixels.
    fn centre(rect: Rect) -> (i32, i32) {
        let middle = |from: f32, to: f32| ((from + to) / 2.0).floor() as i32;
        (middle(rect.x, rect.right), middle(rect.y, rect.bottom))
    }

    #[test]
    fn the_label_shows_seconds_up_to_99_then_minutes_rounded_up() {
        for (seconds, shown) in [
            (0, "0"),
            (1, "1"),
            (9, "9"),
            (10, "10"),
            (99, "99"),
            (100, "2m"),
            (119, "2m"),
            (120, "2m"),
            (121, "3m"),
            (3599, "60m"),
            (3600, "60m"),
            (5940, "99m"),
            (5941, "99m"),
            (u64::MAX, "99m"),
        ] {
            assert_eq!(Label::of(seconds), text(shown), "{seconds} s");
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
            let icon = render(&text("8"), CLICKING, size);
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

    /// The segments, in the usual letters: a at the top, then clockwise,
    /// g in the middle.
    const LETTERS: [(char, Segment); 7] = [
        ('a', Top),
        ('b', UpperRight),
        ('c', LowerRight),
        ('d', Bottom),
        ('e', LowerLeft),
        ('f', UpperLeft),
        ('g', Middle),
    ];

    /// Each digit's lit segments, written out apart from [`DIGITS`].
    const LIT: [&str; 10] = [
        "abcdef", "bc", "abdeg", "abcdg", "bcfg", "acdfg", "acdefg", "abc", "abcdefg", "abcdfg",
    ];

    #[test]
    fn a_digit_lights_its_segments_and_leaves_the_rest_dark() {
        for size in SIZES {
            for (digit, lit) in LIT.iter().enumerate() {
                assert_segments(size, &digit.to_string(), lit);
            }
        }
    }

    /// Checks that `digit`, drawn `size` pixels square, lights the
    /// segments named in `lit` and no others.
    fn assert_segments(size: u16, digit: &str, lit: &str) {
        let cell = single(f32::from(size));
        let icon = render(&text(digit), CLICKING, size);
        for (letter, segment) in LETTERS {
            let colour = if lit.contains(letter) {
                WHITE
            } else {
                CLICKING
            };
            let (x, y) = centre(cell.segment(segment));
            let shown = at(&icon, x, y);
            assert_eq!(shown, opaque(colour), "{digit} {segment:?} at {size} px");
        }
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

    /// The cell of a single digit on an icon `side` pixels wide, as
    /// [`Label::layout`] places it: found from the "8", which lights all of
    /// it, and whose first segment is as thick as every stroke.
    fn single(side: f32) -> Cell {
        let rects = text("8").layout(side);
        let around = bounds(&rects);
        Cell {
            x: around.x,
            y: around.y,
            width: around.right - around.x,
            height: around.bottom - around.y,
            thickness: rects[0].bottom - rects[0].y,
        }
    }

    #[test]
    fn the_label_fills_most_of_the_icon_centred_on_whole_pixels() {
        for size in SIZES {
            for shown in ["7", "88", "5m", "60m"] {
                assert_centred(size, shown);
            }
            // One digit stands no smaller than each of two, and two take
            // most of the icon's height.
            let side = f32::from(size);
            let height = |shown: &str| {
                let around = bounds(&text(shown).layout(side));
                around.bottom - around.y
            };
            assert!(height("7") >= height("88"), "{size} px");
            assert!(height("88") >= 0.6 * side, "{size} px");
        }
    }

    /// Checks that `shown` lies on the icon `size` pixels square, centred
    /// to the pixel, with its edges on whole pixels.
    fn assert_centred(size: u16, shown: &str) {
        let side = f32::from(size);
        let rects = text(shown).layout(side);
        assert!(!rects.is_empty(), "{shown} at {size} px");
        let Rect {
            x: left,
            y: top,
            right,
            bottom,
        } = bounds(&rects);
        assert!(
            left >= 0.0 && right <= side,
            "{shown} at {size} px: {left}..{right}"
        );
        assert!((left - (side - right)).abs() <= 1.0, "{shown} at {size} px");
        assert!((top - (side - bottom)).abs() <= 1.0, "{shown} at {size} px");
        let whole = [left, right, top, bottom]
            .iter()
            .all(|edge| *edge == edge.round());
        assert!(whole, "{shown} at {size} px");
    }

    #[test]
    fn every_character_draws_its_own_shape() {
        let shapes: Vec<Vec<u8>> = "0123456789m"
            .chars()
            .map(|c| render(&text(&c.to_string()), RECORDING, 48).data)
            .collect();
        for (i, shape) in shapes.iter().enumerate() {
            assert_ne!(
                shape,
                &render(&text(""), RECORDING, 48).data,
                "character {i}"
            );
            for other in &shapes[i + 1..] {
                assert_ne!(shape, other, "character {i}");
            }
        }
    }

    #[test]
    fn the_m_has_three_legs_below_a_bar() {
        let cell = Cell {
            x: 0.0,
            y: 0.0,
            width: 14.0,
            height: 20.0,
            thickness: 3.0,
        };
        let rects = cell.minutes();
        let lit = |x: f32, y: f32| rects.iter().any(|r| r.contains(Point { x, y }));
        // Nothing in the upper half; the bar at the middle.
        assert!(!lit(7.0, 4.0));
        assert!(lit(7.0, cell.middle() + 0.5));
        // Three legs to the bottom, with gaps between them.
        let row: Vec<bool> = (0..14).map(|x| lit(x as f32 + 0.5, 19.5)).collect();
        let legs = row.windows(2).filter(|w| w[1] && !w[0]).count() + usize::from(row[0]);
        assert_eq!(legs, 3, "{row:?}");
        assert!(row[0] && row[13]);
    }

    #[test]
    fn an_unknown_character_draws_nothing() {
        assert!(single(48.0).character('x').is_empty());
        assert_eq!(
            render(&text("x"), CLICKING, 22).data,
            render(&text(""), CLICKING, 22).data
        );
    }
}
