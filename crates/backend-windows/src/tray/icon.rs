//! The icon in the taskbar: added, changed and removed through
//! `Shell_NotifyIconW`, by this window and one id (never a GUID, which
//! would tie the icon to the executable's path). The taskbar keeps a copy of
//! each picture, so each is destroyed right after the call that hands it
//! over.
//!
//! Each call waits for Explorer, without a limit on Windows 11: only the
//! shell thread makes them.

use std::io;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFY_ICON_MESSAGE, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIcon, DestroyIcon, HICON};

use super::TrayIcon;
use super::shell::Shown;

/// The icon's id among this window's: it has one.
const ICON_ID: u32 = 1;
/// UTF-16 units the taskbar keeps of the icon's name, its NUL included.
const TIP_UNITS: usize = 128;

/// Adds the icon, then asks for the taskbar's current way of reporting
/// clicks and hovers.
pub(super) fn add(hwnd: HWND, content: &Shown, callback: u32) -> io::Result<()> {
    let icon = Icon::new(&content.icon)?;
    let mut data = shown(hwnd, content, icon.0);
    data.uFlags |= NIF_MESSAGE;
    data.uCallbackMessage = callback;
    notify(NIM_ADD, &data)?;
    versioned(hwnd)
}

/// Asks for `NOTIFYICON_VERSION_4`: clicks and hovers each reported once,
/// with the point they happened at.
pub(super) fn versioned(hwnd: HWND) -> io::Result<()> {
    let mut data = identity(hwnd);
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    notify(NIM_SETVERSION, &data)
}

/// Shows `content`'s picture and name on the icon added before.
pub(super) fn modify(hwnd: HWND, content: &Shown) -> io::Result<()> {
    let icon = Icon::new(&content.icon)?;
    notify(NIM_MODIFY, &shown(hwnd, content, icon.0))
}

/// Removes the icon; one that is gone already (Explorer ended) is no error
/// worth telling.
pub(super) fn delete(hwnd: HWND) {
    let _ = notify(NIM_DELETE, &identity(hwnd));
}

fn identity(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..NOTIFYICONDATAW::default()
    }
}

/// The icon showing `content` with the picture `icon`. Without
/// `NIF_SHOWTIP` the taskbar shows no tooltip of its own: the popup shows
/// every line instead, and the name is for screen readers.
fn shown(hwnd: HWND, content: &Shown, icon: HICON) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        uFlags: NIF_ICON | NIF_TIP,
        hIcon: icon,
        szTip: tip(&content.title),
        ..identity(hwnd)
    }
}

/// `title` as the taskbar keeps it: as many whole characters as fit, the
/// rest cut, never half a character.
fn tip(title: &str) -> [u16; TIP_UNITS] {
    let mut tip = [0; TIP_UNITS];
    let mut used = 0;
    for character in title.chars() {
        let mut units = [0; 2];
        let units = character.encode_utf16(&mut units);
        let end = used + units.len();
        // One unit is kept for the NUL.
        if end >= TIP_UNITS {
            break;
        }
        tip[used..end].copy_from_slice(units);
        used = end;
    }
    tip
}

#[allow(unsafe_code)]
fn notify(message: NOTIFY_ICON_MESSAGE, data: &NOTIFYICONDATAW) -> io::Result<()> {
    // SAFETY: `data` is a fully set `NOTIFYICONDATAW` of its own size,
    // alive for the call; its icon, if any, is alive too.
    if unsafe { Shell_NotifyIconW(message, data) } != 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    Err(match err.raw_os_error() {
        Some(0) | None => io::Error::other("the taskbar refused the icon"),
        Some(_) => err,
    })
}

/// A picture made from RGBA pixels, destroyed when dropped.
struct Icon(HICON);

impl Icon {
    #[allow(unsafe_code)]
    fn new(picture: &TrayIcon) -> io::Result<Icon> {
        let (and, xor) = planes(picture)?;
        let side = i32::from(picture.side);
        // SAFETY: `xor` holds `side` rows of `side` 32-bit pixels, and
        // `and` `side` rows of 1-bit pixels padded to 16 bits, as a 32-bit
        // icon with an AND mask takes them; both alive for the call. No
        // module: the icon is not a resource.
        let icon = unsafe { CreateIcon(null_mut(), side, side, 1, 32, and.as_ptr(), xor.as_ptr()) };
        if icon.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Icon(icon))
    }
}

impl Drop for Icon {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: made by `CreateIcon`, destroyed once; the taskbar keeps a
        // copy of its own.
        unsafe { DestroyIcon(self.0) };
    }
}

/// The icon's AND mask (a pixel with no cover set, for systems that ignore
/// alpha) and its colours as BGRA, from straight RGBA.
fn planes(picture: &TrayIcon) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let side = usize::from(picture.side);
    let (pixels, rest) = picture.rgba.as_chunks::<4>();
    let square = rest.is_empty() && pixels.len() == side * side;
    if side == 0 || !square {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the tray's picture is not square RGBA",
        ));
    }
    let stride = side.div_ceil(16) * 2;
    let mut and = vec![0u8; stride * side];
    let mut xor = Vec::with_capacity(pixels.len() * 4);
    for (index, &[r, g, b, a]) in pixels.iter().enumerate() {
        xor.extend([b, g, r, a]);
        if a == 0 {
            let (y, x) = (index / side, index % side);
            and[y * stride + x / 8] |= 0x80 >> (x % 8);
        }
    }
    Ok((and, xor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_keeps_whole_characters_and_a_nul() {
        let short = tip("Auto clicker: idle");
        let units: Vec<u16> = "Auto clicker: idle".encode_utf16().collect();
        assert_eq!(&short[..units.len()], units.as_slice());
        assert_eq!(short[units.len()], 0);
        // 126 units, then a character of two: it does not fit whole.
        let long = format!("{}{}", "a".repeat(126), '\u{1f600}');
        let cut = tip(&long);
        assert_eq!(cut[125], u16::from(b'a'));
        assert_eq!(cut[126..], [0, 0]);
        let full = tip(&"b".repeat(200));
        assert_eq!(full[126], u16::from(b'b'));
        assert_eq!(full[127], 0);
    }

    #[test]
    fn pixels_become_bgra_and_a_mask_of_the_clear_ones() {
        let mut rgba = vec![0u8; 17 * 17 * 4];
        rgba[..4].copy_from_slice(&[1, 2, 3, 0xff]);
        // The second row's tenth pixel is covered too.
        let second = (17 + 9) * 4;
        rgba[second..second + 4].copy_from_slice(&[0, 0, 0, 0x80]);
        let (and, xor) = planes(&TrayIcon { side: 17, rgba }).unwrap();
        assert_eq!(&xor[..4], &[3, 2, 1, 0xff]);
        // Rows of 17 bits padded to 32; the first pixel is covered.
        assert_eq!(and.len(), 4 * 17);
        assert_eq!(&and[..4], &[0x7f, 0xff, 0x80, 0]);
        assert_eq!(&and[4..8], &[0xff, 0xbf, 0x80, 0]);
        // Not whole pixels, or not a square's worth.
        for bytes in [15, 12] {
            assert!(
                planes(&TrayIcon {
                    side: 2,
                    rgba: vec![0; bytes]
                })
                .is_err(),
                "{bytes}"
            );
        }
    }
}
