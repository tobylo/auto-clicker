//! The modifiers held, as the window's thread saw them with the key message
//! it handles now (`GetKeyState`), for the settings window's capture (WN10).
//! Not winit's: it reports AltGr as no modifier at all, where Windows sends
//! AltGr as Ctrl and Alt, and a hotkey registered on Ctrl+Alt takes it.

use auto_clicker_core::hotkeys::{Modifier, Modifiers};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};

/// The modifiers held with the key message this thread handles now.
#[allow(unsafe_code)]
pub fn window_message_modifiers() -> Modifiers {
    held(|key| {
        // SAFETY: plain call; any virtual key may be asked. The high bit
        // says the key is down.
        unsafe { GetKeyState(i32::from(key)) < 0 }
    })
}

/// The modifiers whose virtual keys `down` says are down.
fn held(down: impl Fn(VIRTUAL_KEY) -> bool) -> Modifiers {
    [
        (VK_CONTROL, Modifier::Ctrl),
        (VK_SHIFT, Modifier::Shift),
        (VK_MENU, Modifier::Alt),
        (VK_LWIN, Modifier::Meta),
        (VK_RWIN, Modifier::Meta),
    ]
    .into_iter()
    .filter(|&(key, _)| down(key))
    .fold(Modifiers::NONE, |all, (_, modifier)| all.with(modifier))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held_with(keys: &[VIRTUAL_KEY]) -> Modifiers {
        held(|key| keys.contains(&key))
    }

    #[test]
    fn altgr_is_ctrl_and_alt_as_windows_sends_it() {
        let ctrl_alt = Modifiers::NONE.with(Modifier::Ctrl).with(Modifier::Alt);
        assert_eq!(held_with(&[VK_CONTROL, VK_MENU]), ctrl_alt);
        assert_eq!(
            held_with(&[VK_SHIFT]),
            Modifiers::NONE.with(Modifier::Shift)
        );
        let meta = Modifiers::NONE.with(Modifier::Meta);
        assert_eq!(held_with(&[VK_LWIN]), meta);
        assert_eq!(held_with(&[VK_RWIN]), meta);
        assert_eq!(held_with(&[]), Modifiers::NONE);
    }
}
