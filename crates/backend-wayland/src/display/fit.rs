//! How large the countdown or the badge is on a screen and where it sits,
//! and its buffer at the screen's scale, with the bounds of O8: a surface's
//! side and a buffer's side have a fixed limit, so a buffer holds 64 MiB at
//! most (a badge's far less), and each size is computed with checked
//! arithmetic. Past a bound, the screen shows nothing, with the reason.

use std::fmt;

use auto_clicker_core::config::Corner;
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
/// The badge's side, in logical pixels (O01 decision 15).
pub(crate) const BADGE: u32 = 40;
/// The badge's distance from the corner's two edges, in logical pixels.
pub(crate) const MARGIN: u32 = 24;
/// The longest side of a badge's buffer, in pixels: a scale of 6.4. Each of
/// its pixels takes 16 samples, so a compositor that names an absurd scale
/// cannot hold the daemon's thread drawing it.
pub(crate) const BADGE_MOST: u32 = 256;

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

/// Where a surface sits on its screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// In the middle, where the compositor centres a surface without an
    /// anchor: the countdown.
    Middle,
    /// In a corner, [`MARGIN`] from its two edges: the badge.
    Corner(Corner),
}

/// What shows on one screen: its size in logical pixels, and its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fit {
    /// The surface: the label and its padding, or the badge.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// The characters' height; the badge's side for a badge.
    pub(crate) characters: u32,
    pub(crate) place: Place,
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
                place: Place::Middle,
            }),
            _ => Err(Missing::Size),
        }
    }

    /// The badge in `corner` of a screen `width` by `height` logical
    /// pixels: always [`BADGE`] square, so a screen's new size keeps its
    /// surface; nothing on a screen too small to hold it and its margins.
    pub(crate) fn badge(corner: Corner, (width, height): (i32, i32)) -> Result<Fit, Missing> {
        let room = i64::from(BADGE + 2 * MARGIN);
        if i64::from(width) < room || i64::from(height) < room {
            return Err(Missing::Size);
        }
        Ok(Fit {
            width: BADGE,
            height: BADGE,
            characters: BADGE,
            place: Place::Corner(corner),
        })
    }

    /// The longest side its buffer may have.
    fn most(&self) -> u32 {
        match self.place {
            Place::Middle => MAX_SIDE,
            Place::Corner(_) => BADGE_MOST,
        }
    }

    /// The buffer for this fit at `scale` 120ths: each side rounded, as
    /// `wp_fractional_scale_v1` says, with the characters' height in its
    /// pixels.
    pub(crate) fn buffer(&self, scale: u32) -> Result<Fit, Missing> {
        let most = self.most();
        let scaled = |side: u32| {
            let side = side.checked_mul(scale)?.checked_add(WHOLE / 2)? / WHOLE;
            (1..=most).contains(&side).then_some(side)
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
                place: self.place,
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
        // A screen wider than any there is, though its height would do;
        // up to the widest one allowed, it shows.
        assert_eq!(fit(5, (65_537, 1440)), Err(Missing::Size));
        assert_eq!(fit(5, (100_000, 1440)), Err(Missing::Size));
        assert!(fit(5, (65_536, 1440)).is_ok());
        // The same for a screen taller than any, its width narrow enough
        // that the label fits.
        assert_eq!(fit(5, (1000, 65_537)), Err(Missing::Size));
        assert_eq!(fit(5, (1000, 100_000)), Err(Missing::Size));
        assert!(fit(5, (1000, 65_536)).is_ok());
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
            place: Place::Middle,
        };
        assert_eq!(square.buffer(WHOLE), Ok(square));
    }

    #[test]
    fn a_badge_is_the_same_size_on_every_screen_in_its_corner() {
        for corner in Corner::ALL {
            for screen in [(1920, 1080), (3440, 1440), (1080, 1920), (88, 88)] {
                let badge = Fit::badge(corner, screen).unwrap();
                assert_eq!((badge.width, badge.height), (BADGE, BADGE), "{screen:?}");
                assert_eq!(badge.place, Place::Corner(corner));
            }
        }
        // A screen that cannot hold it and its margins, or has no size.
        for screen in [(87, 1080), (1920, 87), (0, 0), (-1, 1080), (i32::MIN, 1)] {
            assert_eq!(
                Fit::badge(Corner::TopLeft, screen),
                Err(Missing::Size),
                "{screen:?}"
            );
        }
    }

    #[test]
    fn a_badges_buffer_follows_the_scale_up_to_its_own_limit() {
        let badge = Fit::badge(Corner::TopLeft, (1920, 1080)).unwrap();
        let at = |scale| badge.buffer(scale).map(|fit| (fit.width, fit.height));
        assert_eq!(at(WHOLE), Ok((40, 40)));
        assert_eq!(at(150), Ok((50, 50)));
        assert_eq!(at(192), Ok((64, 64)));
        // 6.4 is the most; past it, nothing, where a countdown would show.
        assert_eq!(at(768), Ok((256, 256)));
        // 40 x 769 / 120 rounds to 256, 40 x 770 / 120 to 257.
        assert_eq!(at(769), Ok((256, 256)));
        assert_eq!(at(770), Err(Missing::Buffer));
        assert_eq!(at(WHOLE * 10), Err(Missing::Buffer));
        assert_eq!(at(u32::MAX), Err(Missing::Buffer));
        assert_eq!(badge.buffer(768).unwrap().place, badge.place);
    }
}
