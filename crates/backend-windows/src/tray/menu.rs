//! The tray's menu: built from the content's entries each time it opens,
//! run in the window procedure (its loop hands the thread's other messages
//! on meanwhile), and destroyed when it closes. A submenu is refilled from
//! the latest content each time it opens, so a list that changed since the
//! menu opened shows.

use std::cell::{Cell, RefCell};
use std::io;
use std::ptr::null;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DeleteMenu, DestroyMenu, EndMenu, GetMenuItemCount,
    GetSystemMetrics, HMENU, MENU_ITEM_FLAGS, MF_BYPOSITION, MF_CHECKED, MF_GRAYED, MF_POPUP,
    MF_SEPARATOR, MF_STRING, PostMessageW, SM_MENUDROPALIGNMENT, SetForegroundWindow,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON, TrackPopupMenu,
    WM_NULL,
};

use super::MenuEntry;
use crate::message::wide;
use crate::message_thread::{WindowEvent, destroy, emit};

thread_local! {
    /// Whether the menu's loop runs.
    static OPEN: Cell<bool> = const { Cell::new(false) };
    /// A quit arrived while it ran: the window goes once it ends.
    static QUIT_AFTER: Cell<bool> = const { Cell::new(false) };
    /// When the menu last closed.
    static CLOSED_AT: Cell<Option<Instant>> = const { Cell::new(None) };
    /// The open menu's submenus, by their place among the content's.
    static SUBMENUS: RefCell<Vec<HMENU>> = const { RefCell::new(Vec::new()) };
}

/// Whether the menu's loop runs.
pub(super) fn is_open() -> bool {
    OPEN.get()
}

/// Opens the menu at `anchor` and runs it until it closes; the command
/// chosen, if any. Tells the handler first that it opens. An open that
/// follows a close within a double click's time is the click that closed
/// it (the taskbar reports it once the menu is gone), and opens nothing.
pub(super) fn run(hwnd: HWND, entries: &[MenuEntry], anchor: (i32, i32)) -> Option<u32> {
    if OPEN.get() || just_closed() {
        return None;
    }
    let menu = match Menu::build(entries) {
        Ok(menu) => menu,
        Err(err) => {
            emit(WindowEvent::TrayMenuFailed(err));
            return None;
        }
    };
    emit(WindowEvent::TrayMenuOpening);
    OPEN.set(true);
    let command = menu.track(hwnd, anchor);
    OPEN.set(false);
    CLOSED_AT.set(Some(Instant::now()));
    SUBMENUS.with_borrow_mut(Vec::clear);
    drop(menu);
    if QUIT_AFTER.replace(false) {
        destroy(hwnd);
        return None;
    }
    command
}

/// Whether the menu closed less than a double click's time ago.
#[allow(unsafe_code)]
fn just_closed() -> bool {
    // SAFETY: no arguments.
    let window = Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }));
    CLOSED_AT
        .get()
        .is_some_and(|closed| closed.elapsed() < window)
}

/// Ends the open menu for a quit; whether one was open.
#[allow(unsafe_code)]
pub(super) fn end_for_quit() -> bool {
    if !OPEN.get() {
        return false;
    }
    QUIT_AFTER.set(true);
    // SAFETY: no arguments; it ends the menu this thread shows.
    unsafe { EndMenu() };
    true
}

/// Refills the submenu `handle` (a `WM_INITMENUPOPUP`'s `wparam`) from
/// `entries`, if it is one of the open menu's.
pub(super) fn refill(handle: WPARAM, entries: &[MenuEntry]) {
    let place = SUBMENUS.with_borrow(|submenus| {
        submenus
            .iter()
            .position(|&submenu| submenu as WPARAM == handle)
            .map(|place| (place, submenus[place]))
    });
    let Some((place, submenu)) = place else {
        return;
    };
    let children = entries
        .iter()
        .filter_map(|entry| match entry {
            MenuEntry::Submenu { children, .. } => Some(children.as_slice()),
            _ => None,
        })
        .nth(place)
        .unwrap_or(&[]);
    clear(submenu);
    // A line that cannot be added leaves the submenu short; the next open
    // builds it again.
    let _ = append_all(submenu, children);
}

/// A menu built from entries, destroyed (with its submenus) when dropped.
struct Menu(HMENU);

impl Menu {
    fn build(entries: &[MenuEntry]) -> io::Result<Menu> {
        SUBMENUS.with_borrow_mut(Vec::clear);
        let menu = Menu(new_popup()?);
        append_all(menu.0, entries)?;
        Ok(menu)
    }

    /// Runs the menu at `anchor`; the command chosen, if any.
    #[allow(unsafe_code)]
    fn track(&self, hwnd: HWND, (x, y): (i32, i32)) -> Option<u32> {
        // SAFETY: the metric's index alone.
        let align = if unsafe { GetSystemMetrics(SM_MENUDROPALIGNMENT) } != 0 {
            TPM_RIGHTALIGN
        } else {
            TPM_LEFTALIGN
        };
        let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | align;
        // SAFETY: `hwnd` is this thread's window; brought to the front, so
        // that a click elsewhere closes the menu (KB135788), and sent a
        // message after, so that the next click on the icon opens it again.
        // `self.0` is a live menu; no rectangle.
        let chosen = unsafe {
            SetForegroundWindow(hwnd);
            let chosen = TrackPopupMenu(self.0, flags, x, y, 0, hwnd, null());
            PostMessageW(hwnd, WM_NULL, 0, 0);
            chosen
        };
        u32::try_from(chosen).ok().filter(|&command| command != 0)
    }
}

impl Drop for Menu {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: built by `build`, destroyed once, with its submenus.
        unsafe { DestroyMenu(self.0) };
    }
}

#[allow(unsafe_code)]
fn new_popup() -> io::Result<HMENU> {
    // SAFETY: no arguments.
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(menu)
}

/// Appends `entries` to `menu`, remembering each submenu.
fn append_all(menu: HMENU, entries: &[MenuEntry]) -> io::Result<()> {
    entries.iter().try_for_each(|entry| append(menu, entry))
}

/// Appends `entry` to `menu`; a submenu is remembered once it is attached,
/// and destroyed when it cannot be.
#[allow(unsafe_code)]
fn append(menu: HMENU, entry: &MenuEntry) -> io::Result<()> {
    match entry {
        MenuEntry::Item {
            label,
            command,
            enabled,
            checked,
        } => {
            let flags = item_flags(*enabled && command.is_some(), *checked == Some(true));
            let id = command.map_or(0, |command| command as usize);
            append_line(menu, flags, id, Some(label))
        }
        MenuEntry::Submenu { label, children } => {
            let submenu = new_popup()?;
            let attached = append_all(submenu, children)
                .and_then(|()| append_line(menu, MF_POPUP, submenu as usize, Some(label)));
            if attached.is_err() {
                // SAFETY: made above and attached to nothing.
                unsafe { DestroyMenu(submenu) };
            } else {
                SUBMENUS.with_borrow_mut(|submenus| submenus.push(submenu));
            }
            attached
        }
        MenuEntry::Separator => append_line(menu, MF_SEPARATOR, 0, None),
    }
}

#[allow(unsafe_code)]
fn append_line(
    menu: HMENU,
    flags: MENU_ITEM_FLAGS,
    id: usize,
    label: Option<&str>,
) -> io::Result<()> {
    let text = label.map(shown);
    let text = text.as_ref().map_or(null(), |text| text.as_ptr());
    // SAFETY: `menu` is a live menu; `text` is NUL-terminated UTF-16 alive
    // for the call, or null for a separator. A submenu, once appended, is
    // destroyed with `menu`.
    if unsafe { AppendMenuW(menu, flags, id, text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn item_flags(enabled: bool, checked: bool) -> MENU_ITEM_FLAGS {
    let mut flags = MF_STRING;
    if !enabled {
        flags |= MF_GRAYED;
    }
    if checked {
        flags |= MF_CHECKED;
    }
    flags
}

/// `label` as a menu shows it as written: an ampersand would mark an access
/// key, so two show as one; a tab would push the rest to the right edge.
fn shown(label: &str) -> Vec<u16> {
    wide(&label.replace('&', "&&").replace('\t', " "))
}

/// Removes every line of `menu`.
#[allow(unsafe_code)]
fn clear(menu: HMENU) {
    // SAFETY: `menu` is a live submenu of the open menu; its lines are
    // items, which `DeleteMenu` removes by position.
    unsafe {
        while GetMenuItemCount(menu) > 0 {
            if DeleteMenu(menu, 0, MF_BYPOSITION) == 0 {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_show_as_written() {
        assert_eq!(shown("Rock & roll\tmix"), wide("Rock && roll mix"));
        assert_eq!(shown("Start clicking"), wide("Start clicking"));
    }

    #[test]
    fn a_submenu_that_cannot_be_attached_is_not_kept() {
        SUBMENUS.with_borrow_mut(Vec::clear);
        let play = MenuEntry::Submenu {
            label: "Play".to_owned(),
            children: vec![MenuEntry::Separator],
        };
        // No menu to attach it to.
        assert!(append(std::ptr::null_mut(), &play).is_err());
        assert!(SUBMENUS.with_borrow(Vec::is_empty));
        let menu = Menu::build(std::slice::from_ref(&play)).unwrap();
        assert_eq!(SUBMENUS.with_borrow(Vec::len), 1);
        drop(menu);
    }

    fn item(label: &str) -> MenuEntry {
        MenuEntry::Item {
            label: label.to_owned(),
            command: Some(1),
            enabled: true,
            checked: None,
        }
    }

    fn play(children: Vec<MenuEntry>) -> MenuEntry {
        MenuEntry::Submenu {
            label: "Play".to_owned(),
            children,
        }
    }

    #[test]
    #[allow(unsafe_code)]
    fn an_opening_submenu_is_refilled_from_the_latest_entries() {
        let built = vec![item("Start"), play(vec![item("one")])];
        let menu = Menu::build(&built).unwrap();
        let submenu = SUBMENUS.with_borrow(|submenus| submenus[0]);
        // SAFETY: a live menu of this test's.
        let count = || unsafe { GetMenuItemCount(submenu) };
        let latest = vec![item("Start"), play(vec![item("one"), item("two")])];
        // Another menu's handle is left alone.
        refill(menu.0 as WPARAM, &latest);
        assert_eq!(count(), 1);
        refill(submenu as WPARAM, &latest);
        assert_eq!(count(), 2);
        // Emptied when the latest has none.
        refill(submenu as WPARAM, &[item("Start")]);
        assert_eq!(count(), 0);
        drop(menu);
    }

    #[test]
    fn a_menu_already_open_or_just_closed_opens_nothing() {
        OPEN.set(true);
        assert_eq!(run(std::ptr::null_mut(), &[], (0, 0)), None);
        // Still open, and not closed by this.
        assert!(OPEN.get());
        assert_eq!(CLOSED_AT.get(), None);
        OPEN.set(false);
        CLOSED_AT.set(Some(Instant::now()));
        assert!(just_closed());
        let opened = Instant::now();
        assert_eq!(run(std::ptr::null_mut(), &[], (0, 0)), None);
        assert!(CLOSED_AT.get().unwrap() < opened);
        CLOSED_AT.set(Instant::now().checked_sub(Duration::from_secs(60)));
        assert!(!just_closed());
        // A quit with no menu open goes at once.
        assert!(!end_for_quit());
        assert!(!QUIT_AFTER.get());
    }

    #[test]
    #[allow(unsafe_code)]
    fn items_show_greyed_without_a_command_and_checked_where_said() {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetMenuState;
        let entries = [
            MenuEntry::Item {
                label: "Settings".to_owned(),
                command: None,
                enabled: true,
                checked: None,
            },
            MenuEntry::Item {
                label: "Rock & roll".to_owned(),
                command: Some(100),
                enabled: true,
                checked: Some(true),
            },
        ];
        let menu = Menu::build(&entries).unwrap();
        // SAFETY: a live menu of this test's, by position.
        let state = |place| unsafe { GetMenuState(menu.0, place, MF_BYPOSITION) };
        assert_ne!(state(0) & MF_GRAYED, 0);
        assert_eq!(state(1) & MF_GRAYED, 0);
        assert_ne!(state(1) & MF_CHECKED, 0);
    }

    #[test]
    fn an_item_is_greyed_when_off_and_checked_when_chosen() {
        assert_eq!(item_flags(true, false), MF_STRING);
        assert_eq!(item_flags(false, false), MF_STRING | MF_GRAYED);
        assert_eq!(item_flags(true, true), MF_STRING | MF_CHECKED);
    }
}
