//! A message for the user from a program that has no window of its own.

use std::iter::once;
use std::ptr::null_mut;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW,
};

/// Shows `text` under `title` with an error icon, and waits until the user
/// closes it. For a program that is about to exit with nothing held.
#[allow(unsafe_code)]
pub fn show_error(title: &str, text: &str) {
    let title = wide(title);
    let text = wide(text);
    // SAFETY: both strings are NUL-terminated UTF-16, alive for the call;
    // no owner window is given.
    unsafe {
        MessageBoxW(
            null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        )
    };
}

/// `text` as NUL-terminated UTF-16. A NUL inside it would end it early, so
/// it is replaced.
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16()
        .map(|unit| if unit == 0 { u16::from(b' ') } else { unit })
        .chain(once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_ends_with_its_only_nul() {
        assert_eq!(wide("Aå"), [0x41, 0xe5, 0]);
        assert_eq!(wide("a\0b"), [0x61, 0x20, 0x62, 0]);
        assert_eq!(wide(""), [0]);
    }
}
