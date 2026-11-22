use super::*;

fn text(text: &str) -> Label {
    Label(text.to_owned())
}

/// Whether the pixel whose top left corner is (x, y) is lit by `rects`.
fn lit(rects: &[Rect], x: f32, y: f32) -> bool {
    let point = Point {
        x: x + 0.5,
        y: y + 0.5,
    };
    rects.iter().any(|rect| rect.contains(point))
}

/// The centre of `rect`, on a whole pixel.
fn centre(rect: Rect) -> (f32, f32) {
    let middle = |from: f32, to: f32| ((from + to) / 2.0).floor();
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
        assert_eq!(Label::of(seconds).text(), shown, "{seconds} s");
    }
}

#[test]
fn only_a_tray_countdown_has_seconds_and_a_colour() {
    use Activity::*;
    for (activity, colour) in [
        (StartingIn(5), CLICKING),
        (PlayingIn(5), CLICKING),
        (RecordingIn(5), RECORDING),
    ] {
        assert_eq!(countdown(activity), Some((5, colour)), "{activity:?}");
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
        assert_eq!(countdown(activity), None, "{activity:?}");
    }
}

/// The segments, in the usual letters: a at the top, then clockwise, g in
/// the middle.
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
    for height in [16.0, 35.0, 360.0] {
        let cell = Cell {
            x: 3.0,
            y: 5.0,
            width: (DIGIT * height).round(),
            height,
            thickness: (THICKNESS * height).round(),
        };
        for (digit, segments) in LIT.iter().enumerate() {
            let rects = text(&digit.to_string()).draw(Point { x: 3.0, y: 5.0 }, height);
            for (letter, segment) in LETTERS {
                let (x, y) = centre(cell.segment(segment));
                assert_eq!(
                    lit(&rects, x, y),
                    segments.contains(letter),
                    "{digit} {segment:?} at {height} px"
                );
            }
        }
    }
}

#[test]
fn the_label_lies_on_whole_pixels_as_wide_as_it_says() {
    for height in [16.0, 35.0, 360.0, 1080.0] {
        for shown in ["7", "88", "5m", "99m"] {
            let label = text(shown);
            let rects = label.draw(Point { x: 10.0, y: 20.0 }, height);
            let left = rects.iter().map(|r| r.x).fold(f32::MAX, f32::min);
            let right = rects.iter().map(|r| r.right).fold(f32::MIN, f32::max);
            let top = rects.iter().map(|r| r.y).fold(f32::MAX, f32::min);
            let bottom = rects.iter().map(|r| r.bottom).fold(f32::MIN, f32::max);
            assert_eq!((left, top), (10.0, 20.0), "{shown} at {height} px");
            assert_eq!(right - left, label.width(height), "{shown} at {height} px");
            assert_eq!(bottom - top, height, "{shown} at {height} px");
            let whole = rects
                .iter()
                .flat_map(|r| [r.x, r.y, r.right, r.bottom])
                .all(|edge| edge == edge.round());
            assert!(whole, "{shown} at {height} px");
            assert!(
                (label.width(height) - label.span() * height).abs() <= 3.0,
                "{shown} at {height} px"
            );
        }
    }
}

#[test]
fn every_character_draws_its_own_shape() {
    let corner = Point { x: 0.0, y: 0.0 };
    let shapes: Vec<Vec<Rect>> = "0123456789m"
        .chars()
        .map(|c| text(&c.to_string()).draw(corner, 48.0))
        .collect();
    for (i, shape) in shapes.iter().enumerate() {
        assert!(!shape.is_empty(), "character {i}");
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
    let lit_in = |pixels: Vec<(f32, f32)>| -> Vec<usize> {
        pixels
            .into_iter()
            .enumerate()
            .filter(|&(_, (x, y))| lit(&rects, x, y))
            .map(|(index, _)| index)
            .collect()
    };
    // The middle bar starts halfway down, less half its thickness.
    assert_eq!(cell.middle(), 8.0);
    // Between the legs, only the bar: two pixels, 0.7 of the stroke.
    let column = lit_in((0..20u8).map(|y| (3.0, f32::from(y))).collect());
    assert_eq!(column, [8, 9]);
    // Three legs as thick as the bar: left, centred and right.
    let row = lit_in((0..14u8).map(|x| (f32::from(x), 19.0)).collect());
    assert_eq!(row, [0, 1, 6, 7, 12, 13]);
}

#[test]
fn an_unknown_character_draws_nothing() {
    let corner = Point { x: 0.0, y: 0.0 };
    assert!(text("x").draw(corner, 48.0).is_empty());
    assert!(text("").draw(corner, 48.0).is_empty());
}

#[test]
fn a_rectangle_grows_on_every_side() {
    let rect = |x, y, width, height| Rect::new(Point { x, y }, Size { width, height });
    let grown = rect(10.0, 20.0, 5.0, 6.0).grown(2.0);
    assert_eq!(grown, rect(8.0, 18.0, 9.0, 10.0));
}
