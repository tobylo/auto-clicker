//! Output on Windows: every click, key, move and scroll goes to `SendInput`,
//! one call per input frame, each event carrying the program's marker.
//!
//! `SendInput` needs no device, so output is opened for each run; a run
//! waits for its deadlines on a high-resolution waitable timer of its own
//! ([`crate::run_timer`]).

mod keys;

use std::io;

use auto_clicker_core::input::{
    AbsolutePosition, Button, Input, InputBackend, InputError, Key, PressState, RunTimer,
};
use windows_sys::Win32::Foundation::SetLastError;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
    MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, XBUTTON1, XBUTTON2,
};

use crate::run_timer::WaitableTimer;
use keys::Code;
pub use keys::key as received_key;

/// The mark on every event the program injects, in its extra information
/// (`dwExtraInfo`), by which the hotkey reader and the recorder tell the
/// program's own events from the user's (docs/decisions/0006, point 3).
/// "ACLK" in ASCII; any non-zero value would do.
pub const MARKER: u32 = 0x4143_4C4B;

/// Output through `SendInput`. Holds nothing: every call is complete in
/// itself.
#[derive(Debug)]
pub struct Output {
    _private: (),
}

impl Output {
    /// The output, wrapped so every press is tracked and released. Opening
    /// cannot fail: `SendInput` needs no device, and whether Windows takes
    /// an event shows only when one is sent.
    pub fn open() -> Input<Output> {
        Input::new(Output { _private: () })
    }
}

impl InputBackend for Output {
    fn button(&mut self, button: Button, state: PressState) -> Result<(), InputError> {
        send(&button_event(button, state))
    }

    /// A key Windows has no key for fails to press, naming its code; its
    /// release succeeds without sending anything, as nothing went down.
    fn key(&mut self, key: Key, state: PressState) -> Result<(), InputError> {
        match (keys::code(key), state) {
            (Some(code), _) => send(&key_event(code, state)),
            (None, PressState::Up) => Ok(()),
            (None, PressState::Down) => Err(InputError::Write(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("Windows has no key for Linux key code {}", key.code()),
            ))),
        }
    }

    fn move_relative(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        send(&mouse_event(MOUSEINPUT {
            dx,
            dy,
            dwFlags: MOUSEEVENTF_MOVE,
            ..MOUSEINPUT::default()
        }))
    }

    fn move_absolute(&mut self, position: AbsolutePosition) -> Result<(), InputError> {
        send(&absolute_event(on_pixels(position, virtual_desktop())))
    }

    fn scroll(&mut self, notches: i32) -> Result<(), InputError> {
        send(&scroll_event(notches))
    }

    fn run_timer(&mut self) -> Result<Box<dyn RunTimer>, InputError> {
        WaitableTimer::start()
            .map(|timer| Box::new(timer) as Box<dyn RunTimer>)
            .map_err(InputError::Timer)
    }
}

/// A button's press or release. Back and forward are the side and extra
/// buttons, Windows' X buttons 1 and 2.
fn button_event(button: Button, state: PressState) -> INPUT {
    let down = state == PressState::Down;
    let pick = |press, release| if down { press } else { release };
    let (flags, data) = match button {
        Button::Left => (pick(MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP), 0),
        Button::Right => (pick(MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP), 0),
        Button::Middle => (pick(MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP), 0),
        Button::Side => (pick(MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP), XBUTTON1),
        Button::Extra => (pick(MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP), XBUTTON2),
    };
    mouse_event(MOUSEINPUT {
        mouseData: u32::from(data),
        dwFlags: flags,
        ..MOUSEINPUT::default()
    })
}

/// A key's press or release, by scan code, or for Pause by virtual key.
fn key_event(code: Code, state: PressState) -> INPUT {
    let up = if state == PressState::Up {
        KEYEVENTF_KEYUP
    } else {
        0
    };
    let key = match code {
        Code::Scan { scan, extended } => KEYBDINPUT {
            wScan: u16::from(scan),
            dwFlags: KEYEVENTF_SCANCODE | if extended { KEYEVENTF_EXTENDEDKEY } else { 0 },
            ..KEYBDINPUT::default()
        },
        Code::Virtual(virtual_key) => KEYBDINPUT {
            wVk: virtual_key,
            ..KEYBDINPUT::default()
        },
    };
    keyboard_event(KEYBDINPUT {
        dwFlags: key.dwFlags | up,
        ..key
    })
}

/// The virtual desktop's width and height: the bounding box of every
/// monitor, whatever its origin. `None` when Windows reports no size.
#[allow(unsafe_code)]
fn virtual_desktop() -> Option<(u32, u32)> {
    // SAFETY: plain queries.
    let (width, height) = unsafe {
        (
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    Some((u32::try_from(width).ok()?, u32::try_from(height).ok()?)).filter(|&(w, h)| w > 0 && h > 0)
}

/// `position` as Windows must be sent it to land on the pixel core means.
///
/// A value `v` stands for the point `v * size / 65536` pixels along an axis
/// `size` pixels long, from the box's left or top edge, and Windows
/// truncates it, landing on `floor(v * size / 65536)`. Core names whole
/// pixel `p` by `round(p * 65536 / size)`, which stands for a point up to
/// half a step (`size / 131072` px) either side of `p`: truncated, it lands
/// a pixel short about half the time (measured in the VM). So each axis is
/// read as the pixel its point falls in once moved on by half a step, and
/// sent as the smallest value that lands there. That keeps whole pixels on
/// `p` and a pixel's middle (a floored recording's, `p + 0.5`) on `p` too.
/// Measured from the box's corner, a negative origin needs nothing more.
/// Without a size, the position goes as it is.
fn on_pixels(position: AbsolutePosition, desktop: Option<(u32, u32)>) -> AbsolutePosition {
    let Some((width, height)) = desktop else {
        return position;
    };
    AbsolutePosition {
        x: snap(position.x, width),
        y: snap(position.y, height),
    }
}

/// One axis of [`on_pixels`]: the pixel `value` stands in, moved on by half
/// a step, `floor((2 * value + 1) * size / 131072)`, and the smallest value
/// Windows truncates onto it. Exact for axes under 32,768 px, where half a
/// step stays under a quarter pixel. In `u64` it cannot overflow: both
/// factors are below 2^32.
fn snap(value: u16, size: u32) -> u16 {
    let size = u64::from(size.max(1));
    let pixel = ((2 * u64::from(value) + 1) * size / 131_072).min(size - 1);
    let lowest = (pixel * 65_536).div_ceil(size);
    u16::try_from(lowest).unwrap_or(u16::MAX)
}

/// A move to `position` on the virtual desktop, which Windows spans with 0
/// to 65535 on each axis.
fn absolute_event(position: AbsolutePosition) -> INPUT {
    let flags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    mouse_event(MOUSEINPUT {
        dx: i32::from(position.x),
        dy: i32::from(position.y),
        dwFlags: flags,
        ..MOUSEINPUT::default()
    })
}

/// One wheel notch in `mouseData` (Windows' `WHEEL_DELTA`).
const NOTCH: i32 = 120;

/// The wheel turned by `notches`, positive up as on Linux.
fn scroll_event(notches: i32) -> INPUT {
    let amount = notches.saturating_mul(NOTCH);
    // `mouseData` carries the signed amount in its bits.
    mouse_event(MOUSEINPUT {
        mouseData: amount.cast_unsigned(),
        dwFlags: MOUSEEVENTF_WHEEL,
        ..MOUSEINPUT::default()
    })
}

/// `mouse` as an input event, with the marker.
fn mouse_event(mouse: MOUSEINPUT) -> INPUT {
    marked(Event::Mouse(mouse))
}

/// `key` as an input event, with the marker.
fn keyboard_event(key: KEYBDINPUT) -> INPUT {
    marked(Event::Keyboard(key))
}

/// One event for `SendInput`.
enum Event {
    Mouse(MOUSEINPUT),
    Keyboard(KEYBDINPUT),
}

/// `event` with the marker in its extra information.
fn marked(event: Event) -> INPUT {
    let extra = marker();
    match event {
        Event::Mouse(mouse) => INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwExtraInfo: extra,
                    ..mouse
                },
            },
        },
        Event::Keyboard(key) => INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    dwExtraInfo: extra,
                    ..key
                },
            },
        },
    }
}

/// [`MARKER`] as `dwExtraInfo` holds it.
fn marker() -> usize {
    // Widening: a u32 fits in a usize on every Windows target this builds for.
    usize::try_from(MARKER).unwrap_or(usize::MAX)
}

/// Sends `event`. Windows refuses an event without saying why when a
/// window of a higher integrity level has the focus (an elevated program,
/// the lock screen or a UAC prompt); that is reported as access denied.
#[allow(unsafe_code)]
fn send(event: &INPUT) -> Result<(), InputError> {
    let size = i32::try_from(size_of::<INPUT>()).unwrap_or(i32::MAX);
    // SAFETY: plain value. Cleared, so a refusal that sets no error is not
    // read as an earlier call's.
    unsafe { SetLastError(0) };
    // SAFETY: `event` points at one initialised INPUT, alive for the call,
    // and `size` is that struct's size.
    let inserted = unsafe { SendInput(1, event, size) };
    if inserted == 1 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    Err(InputError::Write(match error.raw_os_error() {
        Some(0) | None => io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Windows did not take the event (a window of higher integrity has the focus)",
        ),
        Some(_) => error,
    }))
}

#[cfg(test)]
mod device_tests;
#[cfg(test)]
mod tests;
