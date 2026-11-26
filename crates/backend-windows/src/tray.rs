//! The tray icon (M5 brief decisions 4 and 15, amended by WN07): the
//! picture, its name, the hover popup with every line, and the menu. The
//! daemon draws what it shows ([`TrayContent`]) and hands it over with
//! [`Remote::show_tray`](crate::Remote::show_tray); the message thread
//! shows the popup and the menu and reports what the user chose, and
//! decides nothing itself.
//!
//! The message thread makes no shell call: `Shell_NotifyIconW` waits on
//! Explorer without a limit, so the icon itself is kept by a thread of its
//! own ([`shell`]), and a hung Explorer never holds up a lock or a sleep.
//! Nothing here runs inside the handler: content is taken, and the menu
//! runs its loop, in the window procedure, so the handler is free to hear
//! the session's notices meanwhile.

mod icon;
mod menu;
mod popup;
mod shell;

use std::cell::RefCell;
use std::io;
use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Shell::{NIN_POPUPCLOSE, NIN_POPUPOPEN, NIN_SELECT, NINF_KEY};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetSystemMetrics, RegisterWindowMessageW, SM_CXSMICON, WM_ACTIVATE,
    WM_ACTIVATEAPP, WM_APP, WM_CONTEXTMENU, WM_IME_NOTIFY, WM_IME_SETCONTEXT, WM_INITMENUPOPUP,
    WM_KEYFIRST, WM_KEYLAST, WM_KILLFOCUS, WM_NCACTIVATE, WM_NULL, WM_SETFOCUS,
};

use crate::Remote;
use crate::message::wide;
use crate::message_thread::{WindowEvent, emit, take_tray_content};
use shell::{Calls, Outbox, Shell, Shown};

/// Posted by [`Remote::show_tray`](crate::Remote::show_tray): new content
/// waits in the shared slot.
pub(crate) const WM_TRAY_REFRESH: u32 = WM_APP + 2;
/// What the taskbar sends about the icon: a click, a hover.
const WM_TRAY_CALLBACK: u32 = WM_APP + 3;
/// Posted by the shell thread: it has news for the handler.
const WM_TRAY_NEWS: u32 = WM_APP + 4;
/// `NIN_KEYSELECT`: the icon chosen from the keyboard.
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
/// How long the window's end waits for the icon's removal: within the
/// daemon's second for the whole thread. Explorer may be hung; then the
/// icon is left behind and goes when Explorer next looks at it.
const REMOVAL: Duration = Duration::from_millis(500);

/// What the tray shows: drawn by the daemon, shown as given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayContent {
    /// The first line: the icon's name for screen readers (cut to what the
    /// taskbar holds) and the popup's title.
    pub title: String,
    /// The popup's other lines.
    pub details: Vec<String>,
    pub icon: TrayIcon,
    pub menu: Vec<MenuEntry>,
}

/// A square picture, `side` pixels wide ([`icon_side`]): straight RGBA,
/// row by row from the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayIcon {
    pub side: u16,
    pub rgba: Vec<u8>,
}

/// One line of the menu. Labels are plain text: shown as written, with no
/// access keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    /// An item: chosen, it reports its command (`None`: never), and shows
    /// a check where `checked` says.
    Item {
        label: String,
        command: Option<u32>,
        enabled: bool,
        checked: Option<bool>,
    },
    /// A submenu, built afresh from the latest content each time it opens.
    Submenu {
        label: String,
        children: Vec<MenuEntry>,
    },
    Separator,
}

/// The side of the tray's icon, in pixels, at the process's DPI.
#[allow(unsafe_code)]
pub fn icon_side() -> u16 {
    // SAFETY: no arguments but the metric's index.
    let side = unsafe { GetSystemMetrics(SM_CXSMICON) };
    u16::try_from(side)
        .ok()
        .filter(|&side| side > 0)
        .unwrap_or(16)
}

thread_local! {
    /// The latest content.
    static CONTENT: RefCell<Option<TrayContent>> = const { RefCell::new(None) };
    /// The shell thread, while the window lives.
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
    /// What the shell thread has for the handler.
    static OUTBOX: RefCell<Option<Outbox>> = const { RefCell::new(None) };
    /// `TaskbarCreated`'s number; 0 when it could not be registered.
    static TASKBAR_CREATED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Registers for `TaskbarCreated` and starts the shell thread for `hwnd`,
/// reached through `remote`; called once the window exists. A shell thread
/// that cannot start leaves the tray without its icon, and says why.
#[allow(unsafe_code)]
pub(crate) fn register(hwnd: HWND, remote: Remote) {
    let name = wide("TaskbarCreated");
    // SAFETY: `name` is NUL-terminated UTF-16, alive for the call.
    TASKBAR_CREATED.set(unsafe { RegisterWindowMessageW(name.as_ptr()) });
    let outbox = Outbox::default();
    let calls = Win32 {
        window: Target(hwnd),
        remote,
        outbox: outbox.clone(),
    };
    match Shell::spawn(calls) {
        Ok(shell) => {
            SHELL.set(Some(shell));
            OUTBOX.set(Some(outbox));
        }
        Err(err) => emit(WindowEvent::TrayUnshown(err)),
    }
}

/// The tray's part of the window procedure: the answer to one of its own
/// messages, or `None` for the rest.
pub(crate) fn answer(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let taskbar = TASKBAR_CREATED.get();
    match message {
        WM_TRAY_REFRESH => {
            if let Some(content) = take_tray_content() {
                show(content);
            }
        }
        WM_TRAY_NEWS => news(),
        WM_TRAY_CALLBACK => callback(hwnd, wparam, lparam),
        WM_INITMENUPOPUP if menu::is_open() => CONTENT.with_borrow(|content| {
            menu::refill(wparam, content.as_ref().map_or(&[], |c| &c.menu));
        }),
        // Explorer started again: the icon is added afresh, and a popup
        // shown for the old one would hear no leave.
        _ if taskbar != 0 && message == taskbar => {
            popup::hide();
            SHELL.with_borrow(|shell| shell.as_ref().map(Shell::restarted));
        }
        _ => return None,
    }
    Some(0)
}

/// Whether `message`, one the window does not act on, came from the user's
/// hand rather than woke the thread: everything while the menu runs, and
/// the window brought to the front for the menu.
pub(crate) fn quiet(message: u32) -> bool {
    menu::is_open() || users_hand(message)
}

/// Whether `message` comes from the user's hand: the window brought to the
/// front for the menu and losing it again, keys pressed while it was in
/// front, and the message posted after the menu closes.
fn users_hand(message: u32) -> bool {
    matches!(
        message,
        WM_NULL
            | WM_ACTIVATE
            | WM_SETFOCUS
            | WM_KILLFOCUS
            | WM_ACTIVATEAPP
            | WM_NCACTIVATE
            | WM_IME_SETCONTEXT
            | WM_IME_NOTIFY
    ) || (WM_KEYFIRST..=WM_KEYLAST).contains(&message)
}

/// Keeps `content` for the menu and the popup, and hands its picture and
/// name to the shell thread.
fn show(content: TrayContent) {
    let shown = Shown {
        title: content.title.clone(),
        icon: content.icon.clone(),
    };
    CONTENT.set(Some(content));
    popup::refresh();
    SHELL.with_borrow(|shell| shell.as_ref().map(|shell| shell.show(shown)));
}

/// Hands the shell thread's news to the handler. An icon the taskbar
/// refused hears no leave, so its popup hides.
fn news() {
    let news = OUTBOX.with_borrow(|outbox| outbox.as_ref().map(Outbox::take));
    for event in news.into_iter().flatten() {
        if matches!(event, WindowEvent::TrayUnshown(_)) {
            popup::hide();
        }
        emit(event);
    }
}

/// What the taskbar says about the icon: a click or a key opens the menu, a
/// hover shows the popup. The anchor in `wparam` is right for the menu's
/// events only; the popup goes by the pointer.
fn callback(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    // The event is in the low word of `lparam` (`NOTIFYICON_VERSION_4`).
    let event = u32::from(lparam as u16);
    match event {
        WM_CONTEXTMENU | NIN_SELECT | NIN_KEYSELECT => {
            popup::hide();
            let chosen = CONTENT
                .with_borrow(Clone::clone)
                .and_then(|content| menu::run(hwnd, &content.menu, anchor(wparam)));
            if let Some(command) = chosen {
                emit(WindowEvent::TrayChosen(command));
            }
        }
        NIN_POPUPOPEN => popup::show(hwnd),
        NIN_POPUPCLOSE => popup::hide(),
        _ => {}
    }
}

/// The screen point in a callback's `wparam`: x in the low word, y in the
/// high, each signed.
fn anchor(wparam: WPARAM) -> (i32, i32) {
    let x = i32::from(wparam as u16 as i16);
    let y = i32::from((wparam >> 16) as u16 as i16);
    (x, y)
}

/// Whether a quit must wait for the open menu, which is then closed; the
/// window is destroyed once the menu's loop has ended.
pub(crate) fn quit_waits_for_menu() -> bool {
    menu::end_for_quit()
}

/// Removes the icon and the popup, on the window's way out.
pub(crate) fn remove() {
    popup::destroy();
    if let Some(shell) = SHELL.take() {
        shell.remove(REMOVAL);
    }
    OUTBOX.set(None);
    CONTENT.set(None);
}

/// The latest content's title and lines, for the popup.
fn text() -> Option<(String, Vec<String>)> {
    CONTENT.with_borrow(|content| {
        content
            .as_ref()
            .map(|content| (content.title.clone(), content.details.clone()))
    })
}

/// The message thread's window, for the shell thread's calls.
#[derive(Clone, Copy)]
struct Target(HWND);

// SAFETY: a window handle is an identifier, not a pointer this program
// dereferences; the shell thread only names it to the taskbar as the
// icon's owner, which Windows allows from any thread. The window outlives
// the calls: its end waits for the icon's removal, or leaves a hung call
// to end with the process.
#[allow(unsafe_code)]
unsafe impl Send for Target {}

/// The shell thread's calls, made for real.
struct Win32 {
    window: Target,
    remote: Remote,
    outbox: Outbox,
}

impl Calls for Win32 {
    fn add(&mut self, shown: &Shown) -> io::Result<()> {
        icon::add(self.window.0, shown, WM_TRAY_CALLBACK)
    }

    fn modify(&mut self, shown: &Shown) -> io::Result<()> {
        icon::modify(self.window.0, shown)
    }

    fn version(&mut self) -> io::Result<()> {
        icon::versioned(self.window.0)
    }

    fn delete(&mut self) {
        icon::delete(self.window.0);
    }

    #[allow(unsafe_code)]
    fn taskbar_present(&mut self) -> bool {
        let class = wide("Shell_TrayWnd");
        // SAFETY: `class` is NUL-terminated UTF-16 alive for the call; no
        // title. It reads the window list, and waits on no other program.
        !unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) }.is_null()
    }

    fn tell(&mut self, event: WindowEvent) {
        self.outbox.push(event);
        // A window that is gone has no handler left to tell.
        let _ = self.remote.post(WM_TRAY_NEWS, 0, 0);
    }
}

#[cfg(test)]
pub(crate) mod tests;
