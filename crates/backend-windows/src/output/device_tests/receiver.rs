//! The receiving end of the device tests: a plain visible window in a
//! process of its own, so the clicking process has no window, as the
//! daemon has none. It notes every key and mouse message that carries the
//! program's marker, with its arrival time and its extra information, and
//! prints them when its input closes. Input without the marker (someone at
//! the keyboard) is never noted: an event that lost its marker shows as
//! missing.
//!
//! Lines on standard output, after the test harness's own:
//! - `rx ready <hwnd>` once the window is in front;
//! - `rx key <down|up> <µs> <scan> <extended> <vk> <extra>`;
//! - `rx button <down|up> <which> <µs> <extra>`;
//! - `rx wheel <µs> <delta> <extra>`;
//! - `rx end`.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::io::{BufRead, Write as _};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetMessageExtraInfo,
    GetMessageW, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOWNORMAL,
    SetForegroundWindow, ShowWindow, WM_CLOSE, WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};

use crate::message::wide;
use crate::output::{key_event, keys, marker, send};
use auto_clicker_core::input::PressState;

/// The extra information on the receiver's own taps, which, without the
/// marker, it leaves out: they can arrive after it starts noting.
const OWN: usize = 0xF24;

/// Where the window sits: the tests aim the pointer at its middle.
pub(super) const RECT: (i32, i32, i32, i32) = (100, 100, 1200, 800);

/// The longest the receiver lives, should its input never close.
const LIFETIME: Duration = Duration::from_secs(600);

thread_local! {
    static NOTED: RefCell<Noted> = RefCell::new(Noted::default());
}

#[derive(Default)]
struct Noted {
    start: Option<Instant>,
    lines: Vec<String>,
}

fn note(line: impl FnOnce(u128) -> String) {
    if extra() != marker() {
        return;
    }
    NOTED.with(|noted| {
        let mut noted = noted.borrow_mut();
        let micros = noted.start.map_or(0, |start| start.elapsed().as_micros());
        let line = line(micros);
        noted.lines.push(line);
    });
}

#[allow(unsafe_code)]
fn extra() -> usize {
    // SAFETY: no arguments; the extra information of the message being
    // handled.
    unsafe { GetMessageExtraInfo() }.cast_unsigned()
}

fn high_word(value: usize) -> u16 {
    u16::try_from((value >> 16) & 0xffff).unwrap_or(0)
}

#[allow(unsafe_code)]
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let button = |which: &str, state: &str| {
        note(|at| format!("rx button {state} {which} {at} {:#x}", extra()));
    };
    match message {
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
            let state = if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN) {
                "down"
            } else {
                "up"
            };
            let bits = lparam.cast_unsigned();
            let scan = (bits >> 16) & 0xff;
            let extended = (bits >> 24) & 1;
            note(|at| {
                format!(
                    "rx key {state} {at} {scan:#x} {extended} {:#x} {:#x}",
                    wparam,
                    extra()
                )
            });
            // Not handed on: Alt and F10 would enter the menu loop.
            return 0;
        }
        WM_LBUTTONDOWN => button("left", "down"),
        WM_LBUTTONUP => button("left", "up"),
        WM_RBUTTONDOWN => button("right", "down"),
        WM_RBUTTONUP => button("right", "up"),
        WM_MBUTTONDOWN => button("middle", "down"),
        WM_MBUTTONUP => button("middle", "up"),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let state = if message == WM_XBUTTONDOWN {
                "down"
            } else {
                "up"
            };
            let which = format!("x{}", high_word(wparam));
            button(&which, state);
            // Handled: no back or forward command follows.
            return 1;
        }
        WM_MOUSEWHEEL => {
            let delta = high_word(wparam).cast_signed();
            note(|at| format!("rx wheel {at} {delta} {:#x}", extra()));
            return 0;
        }
        WM_DESTROY => {
            // SAFETY: on the window's own thread.
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        _ => {}
    }
    // SAFETY: the arguments this procedure was given.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Runs the receiver: the window, until standard input closes or its
/// lifetime passes, then the lines.
#[allow(unsafe_code)]
pub(super) fn serve() {
    let class = wide("auto-clicker-wn04-receiver");
    // SAFETY: plain window creation on this thread; every pointer is valid
    // for its call.
    let window = unsafe {
        let instance = GetModuleHandleW(null());
        let class_info = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..WNDCLASSW::default()
        };
        assert_ne!(RegisterClassW(&raw const class_info), 0);
        let title = wide("auto-clicker WN04 receiver");
        let (x, y, width, height) = RECT;
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            x,
            y,
            width,
            height,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        assert!(!window.is_null());
        // The first call takes the starter's show state; the second, ours.
        ShowWindow(window, SW_SHOWNORMAL);
        ShowWindow(window, SW_SHOWNORMAL);
        window
    };
    bring_to_front(window);
    NOTED.with(|noted| {
        let mut noted = noted.borrow_mut();
        noted.lines.clear();
        noted.start = Some(Instant::now());
    });
    close_when_input_closes(window);
    println!("rx ready {:#x}", window as usize);
    let _ = std::io::stdout().flush();
    // SAFETY: an all-zero MSG is valid; the loop runs on the window's thread.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&raw mut message, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&raw const message);
        }
    }
    let mut out = String::new();
    NOTED.with(|noted| {
        for line in &noted.borrow().lines {
            let _ = writeln!(out, "{line}");
        }
    });
    println!("{out}rx end");
    let _ = std::io::stdout().flush();
}

/// Windows gives the foreground only to the process that sent the last
/// input, so the receiver taps F24 (no key's name in the program) first.
#[allow(unsafe_code)]
fn bring_to_front(window: HWND) {
    let f24 = auto_clicker_core::input::Key::from_name("f24").unwrap();
    let code = keys::code(f24).unwrap();
    let tap = [PressState::Down, PressState::Up].map(|state| {
        let mut event = key_event(code, state);
        event.Anonymous.ki.dwExtraInfo = OWN;
        event
    });
    for _ in 0..10 {
        for event in &tap {
            let _ = send(event);
        }
        std::thread::sleep(Duration::from_millis(50));
        // SAFETY: a window of this thread.
        unsafe { SetForegroundWindow(window) };
        // SAFETY: no arguments.
        if unsafe { GetForegroundWindow() } == window {
            return;
        }
    }
    panic!("the receiver's window could not come to the front");
}

/// Closes `window` once standard input closes, or at the receiver's
/// lifetime.
#[allow(unsafe_code)]
fn close_when_input_closes(window: HWND) {
    let address = window as usize;
    std::thread::spawn(move || {
        let deadline = Instant::now() + LIFETIME;
        let (closed, wait) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                if line.is_err() {
                    break;
                }
            }
            let _ = closed.send(());
        });
        let _ = wait.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        // SAFETY: posting to a window by its handle; a window that is gone
        // makes the call fail, which is fine.
        unsafe { PostMessageW(address as HWND, WM_CLOSE, 0, 0) };
    });
}
