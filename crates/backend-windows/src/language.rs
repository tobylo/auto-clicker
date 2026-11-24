//! The languages the user reads Windows in, most preferred first.

use std::io;
use std::ptr::null_mut;

use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

/// The user's display languages as language names ("sv-SE", "en-US"), the
/// one Windows shows first.
///
/// # Errors
/// Windows cannot list them, or the list changed between asking for its
/// size and reading it.
#[allow(unsafe_code)]
pub fn display_languages() -> io::Result<Vec<String>> {
    let mut count = 0u32;
    let mut len = 0u32;
    // SAFETY: with no buffer and a length of 0, the call only writes the
    // length the list needs to `len`; `count` and `len` are live and
    // writable.
    let sized =
        unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, null_mut(), &mut len) };
    if sized == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0u16; usize::try_from(len).map_err(io::Error::other)?];
    // SAFETY: `buffer` holds `len` UTF-16 units, the length given, and is
    // alive and writable for the call; `count` and `len` are too.
    let read = unsafe {
        GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, buffer.as_mut_ptr(), &mut len)
    };
    if read == 0 {
        return Err(io::Error::last_os_error());
    }
    let written = usize::try_from(len).map_or(buffer.len(), |len| len.min(buffer.len()));
    Ok(names(&buffer[..written]))
}

/// The names in a Windows list of strings: each ends with a NUL, and an
/// empty one ends the list.
fn names(list: &[u16]) -> Vec<String> {
    list.split(|&unit| unit == 0)
        .take_while(|name| !name.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn a_list_is_read_up_to_its_empty_name() {
        assert_eq!(names(&wide("sv-SE\0en-US\0\0")), ["sv-SE", "en-US"]);
        assert_eq!(names(&wide("sv-SE\0\0de-DE\0\0")), ["sv-SE"]);
        assert_eq!(names(&wide("en-GB")), ["en-GB"]);
        assert!(names(&wide("\0\0")).is_empty());
        assert!(names(&[]).is_empty());
    }

    #[test]
    fn windows_lists_at_least_one_display_language() {
        let languages = display_languages().unwrap();
        assert!(!languages.is_empty());
        for name in languages {
            assert!(!name.is_empty() && !name.contains('\0'), "{name:?}");
        }
    }
}
