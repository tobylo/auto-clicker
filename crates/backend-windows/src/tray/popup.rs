//! The popup that shows every line while the pointer rests on the icon:
//! the taskbar's own tooltip keeps 127 characters, and the tray has more to
//! say. A window of the message thread's own, made the first time it shows,
//! never active and never taking a click (layered and transparent, so a
//! click reaches whatever window is below, another program's included), in
//! the tooltip's colours and the status bar's font. Its text is also its window text, which screen
//! readers and the device tests read.
//!
//! It shows beside the pointer resting on the icon, and hides when the
//! pointer leaves, when the menu opens, when Explorer restarts or refuses
//! the icon (no leave would follow) and when the window goes. It stays on
//! the screen: one taller than the work area shows from the top and loses
//! its last lines, which the log also holds.

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_INFOBK, COLOR_INFOTEXT, CreateFontIndirectW, DEFAULT_GUI_FONT, DT_CALCRECT,
    DT_EDITCONTROL, DT_NOPREFIX, DT_WORDBREAK, DeleteObject, DrawTextW, EndPaint, FW_BOLD,
    FillRect, GetDC, GetMonitorInfoW, GetStockObject, GetSysColor, GetSysColorBrush, HDC, HFONT,
    HGDIOBJ, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, PAINTSTRUCT,
    ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, HWND_TOPMOST,
    LWA_ALPHA, NONCLIENTMETRICSW, RegisterClassW, SPI_GETNONCLIENTMETRICS, SW_HIDE, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowPos, SetWindowTextW, ShowWindow,
    SystemParametersInfoW, UnregisterClassW, WINDOW_EX_STYLE, WM_PAINT, WNDCLASSW, WS_BORDER,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use super::{icon_side, text};
use crate::message::wide;

/// The widest a line runs before it wraps, in pixels.
const WIDTH: i32 = 420;
/// The space around the text, and between the popup and the icon.
const PADDING: i32 = 6;
const GAP: i32 = 8;
/// Never in front of the user's work, never active, never on the taskbar,
/// and never hit: layered and transparent, a click goes to the window
/// below whichever thread owns it (`HTTRANSPARENT` passes it only to the
/// same thread's windows).
const EX_STYLE: WINDOW_EX_STYLE =
    WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT;
/// How the text is laid out: wrapped at words, a word longer than a line
/// broken, and `&` shown as written.
const TEXT: u32 = DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX;

/// Names each thread's popup class apart, so tests can run several.
static CLASSES: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// The popup, once made; null before.
    static POPUP: Cell<HWND> = const { Cell::new(null_mut()) };
    /// Its class's name, while registered.
    static CLASS: RefCell<Option<Vec<u16>>> = const { RefCell::new(None) };
    /// The fonts: the title's and the lines'.
    static FONTS: Cell<Option<Fonts>> = const { Cell::new(None) };
    /// Whether it shows.
    static VISIBLE: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy)]
struct Fonts {
    title: HFONT,
    lines: HFONT,
    /// Whether they were made here, and are deleted with the popup.
    owned: bool,
}

/// Shows the popup, owned by `owner`, beside the pointer on the icon, with
/// the latest lines.
pub(super) fn show(owner: HWND) {
    if POPUP.get().is_null() && !create(owner) {
        return;
    }
    VISIBLE.set(true);
    place();
}

/// Shows the latest lines, if the popup shows.
pub(super) fn refresh() {
    if VISIBLE.get() {
        place();
    }
}

#[allow(unsafe_code)]
pub(super) fn hide() {
    let popup = POPUP.get();
    if VISIBLE.replace(false) && !popup.is_null() {
        // SAFETY: `popup` is this thread's live popup.
        unsafe { ShowWindow(popup, SW_HIDE) };
    }
}

/// Destroys the popup, its fonts and its class, on the window's way out.
#[allow(unsafe_code)]
pub(super) fn destroy() {
    VISIBLE.set(false);
    let popup = POPUP.replace(null_mut());
    if !popup.is_null() {
        // SAFETY: this thread's popup, destroyed once.
        unsafe { DestroyWindow(popup) };
    }
    if let Some(fonts) = FONTS.take()
        && fonts.owned
    {
        // SAFETY: made by `fonts()`, selected into no device context now.
        unsafe {
            DeleteObject(fonts.title as HGDIOBJ);
            DeleteObject(fonts.lines as HGDIOBJ);
        }
    }
    if let Some(class) = CLASS.take() {
        // SAFETY: the class registered by `create`, with no window left.
        unsafe { UnregisterClassW(class.as_ptr(), GetModuleHandleW(null())) };
    }
}

/// Makes the popup, hidden; whether it exists. A popup that cannot be made
/// leaves nothing behind, and the next hover tries again.
#[allow(unsafe_code)]
fn create(owner: HWND) -> bool {
    let n = CLASSES.fetch_add(1, Ordering::Relaxed);
    let class = wide(&format!(
        "auto-clicker.tray-popup.{}.{n}",
        std::process::id()
    ));
    // SAFETY: a null name asks for this program's own module.
    let instance = unsafe { GetModuleHandleW(null()) };
    let window_class = WNDCLASSW {
        style: CS_DROPSHADOW,
        lpfnWndProc: Some(popup_procedure),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        ..WNDCLASSW::default()
    };
    // SAFETY: fully set; its name is NUL-terminated UTF-16 kept in `CLASS`
    // until the class is unregistered.
    if unsafe { RegisterClassW(&window_class) } == 0 {
        return false;
    }
    // SAFETY: the class was registered above; `owner` is this thread's
    // window, so the popup goes with it and never shows on the taskbar.
    let popup = unsafe {
        CreateWindowExW(
            EX_STYLE,
            class.as_ptr(),
            null(),
            WS_POPUP | WS_BORDER,
            0,
            0,
            0,
            0,
            owner,
            null_mut(),
            instance,
            null(),
        )
    };
    if popup.is_null() {
        // SAFETY: the class registered above, with no window.
        unsafe { UnregisterClassW(class.as_ptr(), instance) };
        return false;
    }
    // SAFETY: `popup` is the layered window made above; fully opaque.
    unsafe { SetLayeredWindowAttributes(popup, 0, u8::MAX, LWA_ALPHA) };
    CLASS.set(Some(class));
    POPUP.set(popup);
    FONTS.set(Some(fonts()));
    true
}

/// The status bar's font, and a bold one for the title; the stock font
/// for both when the system will not say.
#[allow(unsafe_code)]
fn fonts() -> Fonts {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
        ..NONCLIENTMETRICSW::default()
    };
    // SAFETY: `metrics` is a live `NONCLIENTMETRICSW` whose size is set, as
    // the call fills it.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast(),
            0,
        )
    };
    let mut bold = metrics.lfStatusFont;
    bold.lfWeight = FW_BOLD as i32;
    // SAFETY: both are font descriptions read above.
    let (lines, title) = unsafe {
        (
            CreateFontIndirectW(&metrics.lfStatusFont),
            CreateFontIndirectW(&bold),
        )
    };
    let made = [lines, title].iter().all(|font| !font.is_null());
    if read != 0 && made {
        return Fonts {
            title,
            lines,
            owned: true,
        };
    }
    // SAFETY: deleting a null handle only fails; a made one is unused.
    unsafe {
        DeleteObject(lines as HGDIOBJ);
        DeleteObject(title as HGDIOBJ);
    }
    // SAFETY: a stock object, never deleted.
    let stock = unsafe { GetStockObject(DEFAULT_GUI_FONT) } as HFONT;
    Fonts {
        title: stock,
        lines: stock,
        owned: false,
    }
}

/// Sizes the popup to the latest lines, puts it beside the icon and shows
/// it, drawn afresh.
#[allow(unsafe_code)]
fn place() {
    let popup = POPUP.get();
    let Some((title, lines)) = text() else {
        return hide();
    };
    let body = lines.join("\n");
    let all = wide(&format!("{title}\n{body}"));
    // SAFETY: `popup` is this thread's live popup; `all` NUL-terminated
    // UTF-16 alive for the call.
    unsafe { SetWindowTextW(popup, all.as_ptr()) };
    let (width, height) = measure(popup, &title, &body);
    let (x, y) = position(width, height, anchor(), work_area());
    // SAFETY: `popup` is live; it is shown without being activated, and
    // repainted whole.
    unsafe {
        SetWindowPos(
            popup,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        InvalidateRect(popup, null(), 1);
    }
}

/// The popup's outer size for `title` over `body`.
#[allow(unsafe_code)]
fn measure(popup: HWND, title: &str, body: &str) -> (i32, i32) {
    // SAFETY: `popup` is live; its device context is released below.
    let dc = unsafe { GetDC(popup) };
    let (title, body) = layout(dc, title, body);
    // SAFETY: from `GetDC` above.
    unsafe { ReleaseDC(popup, dc) };
    let width = title.right.max(body.right) + 2 * PADDING + 2;
    (width, body.bottom + PADDING + 2)
}

/// Where the title and the body go inside the popup, measured on `dc`.
fn layout(dc: HDC, title: &str, body: &str) -> (RECT, RECT) {
    let fonts = FONTS.get();
    let title = text_rect(dc, fonts.map(|f| f.title), title, PADDING);
    let body = text_rect(dc, fonts.map(|f| f.lines), body, title.bottom + 2);
    (title, body)
}

/// The rectangle `text` takes in `font` at most [`WIDTH`] wide, from `top`.
#[allow(unsafe_code)]
fn text_rect(dc: HDC, font: Option<HFONT>, text: &str, top: i32) -> RECT {
    let mut rect = RECT {
        left: PADDING,
        top,
        right: PADDING + WIDTH,
        bottom: top,
    };
    let text = wide(text);
    // SAFETY: `dc` is a live device context; the old font goes back after;
    // `text` is NUL-terminated (length -1) and alive for the call.
    unsafe {
        let old = font.map(|font| SelectObject(dc, font as HGDIOBJ));
        DrawTextW(dc, text.as_ptr(), -1, &mut rect, DT_CALCRECT | TEXT);
        if let Some(old) = old {
            SelectObject(dc, old);
        }
    }
    rect
}

/// Where the icon is, as near as the pointer resting on it says: asking
/// the taskbar would wait on Explorer, which the message thread never does.
#[allow(unsafe_code)]
fn anchor() -> RECT {
    let mut point = POINT::default();
    // SAFETY: `point` is a live `POINT` for the call to fill.
    unsafe { GetCursorPos(&mut point) };
    let half = i32::from(icon_side()) / 2;
    RECT {
        left: point.x - half,
        top: point.y - half,
        right: point.x + half,
        bottom: point.y + half,
    }
}

/// The work area (the screen less the taskbar) of the monitor the icon is
/// on.
#[allow(unsafe_code)]
fn work_area() -> RECT {
    let icon = anchor();
    let centre = POINT {
        x: icon.left + (icon.right - icon.left) / 2,
        y: icon.top + (icon.bottom - icon.top) / 2,
    };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };
    // SAFETY: the nearest monitor always exists; `info`'s size is set.
    unsafe {
        GetMonitorInfoW(
            MonitorFromPoint(centre, MONITOR_DEFAULTTONEAREST),
            &mut info,
        )
    };
    info.rcWork
}

/// Where a popup `width` by `height` goes beside `icon`: centred on it,
/// above it when it fits there, else below, and inside `work`, its top
/// edge first when it is taller.
fn position(width: i32, height: i32, icon: RECT, work: RECT) -> (i32, i32) {
    let centre = icon.left + (icon.right - icon.left) / 2;
    let above = icon.top - GAP - height;
    let y = if above >= work.top {
        above
    } else {
        (icon.bottom + GAP).min(work.bottom - height).max(work.top)
    };
    let x = (centre - width / 2).min(work.right - width).max(work.left);
    (x, y)
}

/// The popup's window procedure: it paints, and leaves the rest to
/// Windows. A panic is not let out of it.
#[allow(unsafe_code)]
unsafe extern "system" fn popup_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let answered = catch_unwind(AssertUnwindSafe(|| match message {
        WM_PAINT => {
            paint(hwnd);
            Some(0)
        }
        _ => None,
    }));
    match answered {
        Ok(Some(result)) => result,
        // SAFETY: the arguments are the ones Windows passed in.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Paints the latest lines: the title in bold, then the rest.
#[allow(unsafe_code)]
fn paint(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: `hwnd` is the popup, painting in its `WM_PAINT`; ended below.
    let dc = unsafe { BeginPaint(hwnd, &mut paint) };
    if let Some((title, lines)) = text() {
        let body = lines.join("\n");
        let (title_rect, body_rect) = layout(dc, &title, &body);
        let fonts = FONTS.get();
        // SAFETY: `dc` is the popup's paint context; the brush is a system
        // one, never deleted.
        unsafe {
            FillRect(dc, &paint.rcPaint, GetSysColorBrush(COLOR_INFOBK));
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, GetSysColor(COLOR_INFOTEXT));
        }
        draw(dc, fonts.map(|f| f.title), &title, title_rect);
        draw(dc, fonts.map(|f| f.lines), &body, body_rect);
    }
    // SAFETY: pairs with `BeginPaint` above.
    unsafe { EndPaint(hwnd, &paint) };
}

#[allow(unsafe_code)]
fn draw(dc: HDC, font: Option<HFONT>, text: &str, mut rect: RECT) {
    let text = wide(text);
    // SAFETY: as in `text_rect`, drawing instead of measuring.
    unsafe {
        let old = font.map(|font| SelectObject(dc, font as HGDIOBJ));
        DrawTextW(dc, text.as_ptr(), -1, &mut rect, TEXT);
        if let Some(old) = old {
            SelectObject(dc, old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn the_popup_stands_above_the_icon_inside_the_work_area() {
        let work = rect(0, 0, 1920, 1032);
        // Above an icon on a bottom taskbar, centred on it.
        let icon = rect(1700, 1040, 1732, 1080);
        assert_eq!(position(200, 100, icon, work), (1616, 932));
        // Pushed left off the screen's right edge.
        assert_eq!(position(500, 100, icon, work).0, 1420);
        // Below an icon on a top taskbar, where above does not fit.
        let top = rect(1700, 0, 1732, 40);
        let work = rect(0, 48, 1920, 1080);
        assert_eq!(position(200, 100, top, work), (1616, 48));
        // Taller than the work area: from its top, the title first.
        assert_eq!(position(200, 2000, top, work), (1616, 48));
    }
}
