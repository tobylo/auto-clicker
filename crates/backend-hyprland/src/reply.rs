//! Hyprland's answer to `cursorpos`, parsed once into a validated type.
//!
//! Hyprland 0.57.2 answers `x, y`: the pointer's global position in logical
//! pixels, floored, as two signed decimal integers, and nothing else, no
//! newline (`src/ipc/s1/Commands.cpp:1384-1387`). Only that form is read:
//! no sign before a positive number, no leading zero, no space but the one
//! after the comma.

/// Where the pointer is in Hyprland's own coordinates: logical pixels,
/// floored, from Hyprland's origin, which the layout's box may lie away
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Global {
    pub(crate) x: i32,
    pub(crate) y: i32,
}

/// `reply` as a position; `None` when it is not exactly `x, y`.
pub(crate) fn parse(reply: &[u8]) -> Option<Global> {
    let text = std::str::from_utf8(reply).ok()?;
    let (x, y) = text.split_once(", ")?;
    Some(Global {
        x: whole(x)?,
        y: whole(y)?,
    })
}

/// A decimal integer as Hyprland prints one.
fn whole(text: &str) -> Option<i32> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let canonical = match digits.as_bytes() {
        [] => false,
        [b'0'] => digits == text,
        [b'0', ..] => false,
        bytes => bytes.iter().all(u8::is_ascii_digit),
    };
    canonical.then(|| text.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_hyprlands_own_form_is_read() {
        let read = [
            ("640, 400", Some((640, 400))),
            ("-1920, -300", Some((-1920, -300))),
            ("0, 0", Some((0, 0))),
            ("2147483647, -2147483648", Some((i32::MAX, i32::MIN))),
        ];
        for (reply, expected) in read {
            let parsed = parse(reply.as_bytes()).map(|global| (global.x, global.y));
            assert_eq!(parsed, expected, "{reply:?}");
        }
        let refused = [
            "",
            "640",
            "640,400",
            "640,  400",
            " 640, 400",
            "640, 400\n",
            "640, 400, 1",
            "+640, 400",
            "0640, 400",
            "-0, 400",
            "640.5, 400",
            "2147483648, 0",
            "x, y",
            "-, 4",
            "ok",
            "640, 4\u{0}",
        ];
        for reply in refused {
            assert_eq!(parse(reply.as_bytes()), None, "{reply:?}");
        }
        assert_eq!(parse(&[0xff, b',', b' ', b'1']), None);
    }
}
