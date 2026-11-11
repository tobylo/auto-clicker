//! What the scripts report, parsed once into the types a recording holds
//! (brief I3).
//!
//! Every value travels as text: how KWin marshals a script's numbers is up
//! to Qt, while text always arrives as a D-Bus string. A position is
//! `"x y"` and a layout is one `"x y width height scale"` per output, joined
//! by `;`, all in logical px. Numbers are plain decimal digits; anything else
//! does not parse.

use std::time::Duration;

use auto_clicker_core::recording::{Layout, MAX_OUTPUTS, Output, Point, Rect};

use crate::token::Token;

/// The longest report text read: 16 outputs of five numbers fit many times
/// over.
pub(crate) const MAX_REPORT_BYTES: usize = 4096;

/// Digits in a number before it stops being one: `u32::MAX` has 10.
const MAX_DIGITS: usize = 10;

/// One call from a script, stamped when it arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Report {
    /// On the event clock.
    pub(crate) time: Duration,
    /// The token of the load that sent it.
    pub(crate) token: Token,
    pub(crate) body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Body {
    /// The pointer is here.
    Position(Point),
    /// The screens are these; `None` when the report was no layout (I1),
    /// which leaves the layout unknown.
    Layout(Option<Layout>),
}

/// A position report, or `None` when it is none.
pub(crate) fn point(text: &str) -> Option<Point> {
    let [x, y] = numbers(text)?;
    Some(Point { x, y })
}

/// A layout report; `None` when it is no layout.
pub(crate) fn layout(text: &str) -> Option<Layout> {
    if text.len() > MAX_REPORT_BYTES {
        return None;
    }
    let mut outputs = Vec::new();
    for part in text.split(';') {
        if outputs.len() == MAX_OUTPUTS {
            return None;
        }
        outputs.push(output(part)?);
    }
    Layout::new(outputs).ok()
}

/// One output: `"x y width height scale"`.
fn output(text: &str) -> Option<Output> {
    let (rect, scale) = text.rsplit_once(' ')?;
    let [x, y, width, height] = numbers(rect)?;
    let rect = Rect {
        x,
        y,
        width,
        height,
    };
    Output::new(rect, scale_milli(scale)?).ok()
}

/// `N` whole numbers separated by single spaces.
fn numbers<const N: usize>(text: &str) -> Option<[u32; N]> {
    let mut parts = text.split(' ');
    let mut values = [0; N];
    for value in &mut values {
        *value = number(parts.next()?)?;
    }
    parts.next().is_none().then_some(values)
}

/// A whole number: decimal digits only, no sign.
fn number(text: &str) -> Option<u32> {
    let fits = text.len() <= MAX_DIGITS;
    (fits && is_digits(text)).then(|| text.parse().ok())?
}

/// One or more decimal digits and nothing else.
fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// A scale such as `1`, `1.25` or `1.2000000476837158`, in thousandths,
/// rounded; `None` unless it is digits with at most one point, and at most
/// 1000.
fn scale_milli(text: &str) -> Option<u32> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, "0"));
    let whole = number(whole).filter(|&whole| whole <= 1000)?;
    if !is_digits(fraction) {
        return None;
    }
    // Three digits and a fourth for rounding are all a thousandth needs.
    let mut digits = fraction.bytes().map(|b| u32::from(b - b'0')).chain([0; 4]);
    let mut milli = whole.checked_mul(1000)?;
    for place in [100, 10, 1] {
        milli = milli.checked_add(digits.next()? * place)?;
    }
    let round_up = digits.next()? >= 5;
    milli.checked_add(u32::from(round_up))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_position_is_two_whole_numbers() {
        assert_eq!(point("1024 640"), Some(Point { x: 1024, y: 640 }));
        assert_eq!(point("0 0"), Some(Point { x: 0, y: 0 }));
        for refused in [
            "",
            "1024",
            "1024 640 1",
            "-1 5",
            "+1 5",
            "1.5 2",
            "1  2",
            " 1 2",
            "1 2 ",
            "x y",
            "4294967296 0",
            "99999999999 0",
        ] {
            assert_eq!(point(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn a_layout_is_every_output_with_its_scale() {
        let two = layout("0 0 2048 1280 1.25;2048 0 3440 1440 1").unwrap();
        let rects: Vec<Rect> = two.outputs().iter().map(|o| o.rect()).collect();
        let scales: Vec<u32> = two.outputs().iter().map(|o| o.scale_milli()).collect();
        assert_eq!(rects, [rect(0, 0, 2048, 1280), rect(2048, 0, 3440, 1440)]);
        assert_eq!(scales, [1250, 1000]);
    }

    #[test]
    fn scales_round_to_thousandths() {
        for (text, milli) in [
            ("1", 1000),
            ("1.5", 1500),
            ("1.25", 1250),
            ("1.2000000476837158", 1200),
            ("1.3333333", 1333),
            ("1.6666667", 1667),
            ("0.9995", 1000),
            ("1000", 1_000_000),
        ] {
            assert_eq!(scale_milli(text), Some(milli), "{text}");
        }
        for refused in ["", ".5", "1.", "1.2.3", "-1", "1e3", "1001", "1,5", " 1"] {
            assert_eq!(scale_milli(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn what_is_no_layout_is_refused() {
        let sixteen = vec!["0 0 10 10 1"; MAX_OUTPUTS].join(";");
        assert!(layout(&sixteen).is_some());
        let seventeen = vec!["0 0 10 10 1"; MAX_OUTPUTS + 1].join(";");
        for refused in [
            "",
            "0 0 2048 1280",
            "0 0 2048 1280 1.25;",
            "0 0 0 1280 1",
            "0 0 2048 1280 0",
            "10 0 2048 1280 1",
            "0 0 2048 1280 1;;0 0 5 5 1",
            "-3440 0 3440 1440 1;0 0 2048 1280 1.25",
            seventeen.as_str(),
        ] {
            assert_eq!(layout(refused), None, "{refused:?}");
        }
        let long = format!("0 0 10 10 1{}", " ".repeat(MAX_REPORT_BYTES));
        assert_eq!(layout(&long), None);
    }
}
