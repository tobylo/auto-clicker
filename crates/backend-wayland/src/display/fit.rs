//! How large the countdown is on a screen, and its buffer at the screen's
//! scale, with the bounds of O8: a surface's side and a buffer's side have a
//! fixed limit, so a buffer holds 64 MiB at most, and each size is computed
//! with checked arithmetic. Past a bound, the screen shows nothing, with the
//! reason.

use std::fmt;

use auto_clicker_core::digits::Label;

/// The characters' height, as a share of the screen's logical height.
const HEIGHT: f32 = 0.25;
/// The widest the label and its padding may be, as a share of the
/// screen's logical width: "99m" fits a portrait screen.
const WIDTH: f32 = 0.9;
/// The space around the label, as a share of the characters' height: it
/// holds the outline.
pub(crate) const PAD: f32 = 0.25;
/// The longest side of a surface or a buffer, in pixels: an 8K screen's
/// countdown fits several times over, and a buffer [`MAX_SIDE`] square
/// holds 64 MiB.
pub(crate) const MAX_SIDE: u32 = 4096;
/// A fractional scale's unit: `wp_fractional_scale_v1` speaks in 120ths.
pub(crate) const WHOLE: u32 = 120;

/// Why a screen shows nothing: kept in the display's state for that
/// screen, and logged (O3, O8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Missing {
    /// The screen is past the most a layout may hold (`MAX_OUTPUTS`).
    TooMany,
    /// The screen's size gives no countdown, or one past [`MAX_SIDE`].
    Size,
    /// The buffer's size at the screen's scale overflows, or is past
    /// [`MAX_SIDE`].
    Buffer,
    /// The compositor closed the surface.
    Closed,
    /// The compositor did not answer the surface in time.
    Late,
    /// The buffer's memory could not be made.
    Memory,
}

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Missing::TooMany => "more screens than a layout may hold",
            Missing::Size => "the screen's size is past the limit",
            Missing::Buffer => "the buffer's size at the screen's scale is past the limit",
            Missing::Closed => "the desktop closed its surface",
            Missing::Late => "the desktop did not answer in time",
            Missing::Memory => "its buffer's memory could not be made",
        })
    }
}

/// The countdown's size on one screen, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fit {
    /// The surface: the label and its padding.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// The characters' height.
    pub(crate) characters: u32,
}

impl Fit {
    /// `label` on a screen `width` by `height` logical pixels: characters
    /// a quarter of its height, as small as it takes for the label and its
    /// padding to fit [`WIDTH`] of its width.
    pub(crate) fn of(label: &Label, (width, height): (i32, i32)) -> Result<Fit, Missing> {
        let screen = |side: i32| u32::try_from(side).ok().filter(|&side| side > 0);
        let (Some(width), Some(height)) = (screen(width), screen(height)) else {
            return Err(Missing::Size);
        };
        if width > MAX_SIDE * 16 || height > MAX_SIDE * 16 {
            return Err(Missing::Size);
        }
        // Both below 2^17: exact as floats.
        let (width, height) = (width as f32, height as f32);
        let fits = (WIDTH * width / (label.span() + 2.0 * PAD)).floor();
        let characters = (HEIGHT * height).round().min(fits);
        let pad = (characters * PAD).round();
        let sides = (label.width(characters) + 2.0 * pad, characters + 2.0 * pad);
        let side = |side: f32| {
            // Whole and below 2^18 here: exact, and in range.
            let side = side as u32;
            (1..=MAX_SIDE).contains(&side).then_some(side)
        };
        match (side(sides.0), side(sides.1), side(characters)) {
            (Some(width), Some(height), Some(characters)) => Ok(Fit {
                width,
                height,
                characters,
            }),
            _ => Err(Missing::Size),
        }
    }

    /// The buffer for this fit at `scale` 120ths: each side rounded, as
    /// `wp_fractional_scale_v1` says, with the characters' height in its
    /// pixels.
    pub(crate) fn buffer(&self, scale: u32) -> Result<Fit, Missing> {
        let scaled = |side: u32| {
            let side = side.checked_mul(scale)?.checked_add(WHOLE / 2)? / WHOLE;
            (1..=MAX_SIDE).contains(&side).then_some(side)
        };
        match (
            scaled(self.width),
            scaled(self.height),
            scaled(self.characters),
        ) {
            (Some(width), Some(height), Some(characters)) => Ok(Fit {
                width,
                height,
                characters,
            }),
            _ => Err(Missing::Buffer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fit(seconds: u64, screen: (i32, i32)) -> Result<Fit, Missing> {
        Fit::of(&Label::of(seconds), screen)
    }

    #[test]
    fn the_characters_are_a_quarter_of_the_screens_height() {
        let five = fit(5, (3440, 1440)).unwrap();
        assert_eq!(five.characters, 360);
        // A pad of a quarter of the characters on each side.
        assert_eq!(five.height, 540);
        assert_eq!(five.width, 198 + 180);
        let widest = fit(5941, (3440, 1440)).unwrap();
        assert_eq!((widest.characters, widest.height), (360, 540));
        assert_eq!(widest.width, 914);
    }

    #[test]
    fn a_narrow_screen_makes_the_label_smaller_to_fit() {
        // A portrait screen: "99m" at a quarter of 1920 would be 1,219 px.
        let widest = fit(5941, (1080, 1920)).unwrap();
        assert!(widest.width <= 972 + 2, "{widest:?}");
        assert!(widest.characters < 480, "{widest:?}");
        // One digit still fits at a quarter of the height.
        assert_eq!(fit(5, (1080, 1920)).unwrap().characters, 480);
    }

    #[test]
    fn a_screen_with_no_size_or_past_the_limit_shows_nothing() {
        for screen in [(0, 1440), (3440, 0), (-1, 1440), (3440, -1440)] {
            assert_eq!(fit(5, screen), Err(Missing::Size), "{screen:?}");
        }
        // A tall one gives a surface past the limit.
        assert_eq!(fit(5, (3440, 20_000)), Err(Missing::Size));
        assert_eq!(fit(5, (i32::MAX, i32::MAX)), Err(Missing::Size));
        // So small that the characters round to nothing.
        assert_eq!(fit(5, (3440, 1)), Err(Missing::Size));
    }

    #[test]
    fn the_buffer_is_the_logical_size_times_the_scale_rounded() {
        let five = fit(5, (3440, 1440)).unwrap();
        assert_eq!(five.buffer(WHOLE), Ok(five));
        let at_125 = five.buffer(150).unwrap();
        // 378 x 1.25 = 472.5, rounded half up; 540 x 1.25 = 675.
        assert_eq!((at_125.width, at_125.height), (473, 675));
        assert_eq!(at_125.characters, 450);
        let at_2 = five.buffer(240).unwrap();
        assert_eq!((at_2.width, at_2.height), (756, 1080));
    }

    #[test]
    fn a_buffer_past_the_limit_or_that_would_overflow_is_refused() {
        let five = fit(5, (3440, 1440)).unwrap();
        // 540 x 10 is past the longest side.
        assert_eq!(five.buffer(WHOLE * 10), Err(Missing::Buffer));
        // The multiplication itself overflows.
        assert_eq!(five.buffer(u32::MAX), Err(Missing::Buffer));
        assert_eq!(five.buffer(0), Err(Missing::Buffer));
        // The longest side itself is allowed.
        let square = Fit {
            width: MAX_SIDE,
            height: MAX_SIDE,
            characters: 1,
        };
        assert_eq!(square.buffer(WHOLE), Ok(square));
    }
}
