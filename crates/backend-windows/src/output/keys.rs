//! Which key Windows is sent for each Linux key code, and back.
//!
//! Keys stay Linux key codes everywhere in the program (docs/decisions/0006,
//! point 1) and are sent by scan code, so a game reading scan codes or raw
//! input sees a real keyboard's key whatever the layout. The table joins two
//! sources on the USB HID usage each key has:
//!
//! - Microsoft's "Scan 1 Make" codes, in "About Keyboard Input", section
//!   "Scan codes" (learn.microsoft.com/windows/win32/inputdev/about-keyboard-input),
//!   where an `E0` prefix is the extended flag;
//! - Linux's usage to key code tables, in `drivers/hid/hid-input.c`
//!   (`hid_keyboard` for the keyboard page, `hidinput_configure_usage` for
//!   the consumer and generic desktop pages).
//!
//! Not QEMU's keycodemapdb "AT set1" column, which follows Linux's own AT
//! driver: it gives PrintScreen as 0x54 and F13 as 0x5d, where Windows has
//! `E0 37` and 0x64.
//!
//! Two keys are sent otherwise, as measured in X03 (0006 point 1): NumLock
//! goes as 0x45 without the extended flag, and Pause by its virtual key
//! alone, since its scan code is the three bytes `E1 1D 45`. A key code
//! missing here has no key on Windows.

use auto_clicker_core::input::Key;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_PAUSE;

/// What Windows is sent for a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Code {
    /// A set-1 scan code, with the extended flag for an `E0` key.
    Scan { scan: u8, extended: bool },
    /// A virtual key, for the one key a scan code cannot be sent for.
    Virtual(u16),
}

const fn plain(scan: u8) -> Code {
    Code::Scan {
        scan,
        extended: false,
    }
}

const fn extended(scan: u8) -> Code {
    Code::Scan {
        scan,
        extended: true,
    }
}

/// Every Linux key code Windows has a key for, in code order, with the
/// key's Linux name.
#[rustfmt::skip]
const TABLE: &[(u16, Code)] = &[
    (1, plain(0x01)), // esc
    (2, plain(0x02)), // 1
    (3, plain(0x03)), // 2
    (4, plain(0x04)), // 3
    (5, plain(0x05)), // 4
    (6, plain(0x06)), // 5
    (7, plain(0x07)), // 6
    (8, plain(0x08)), // 7
    (9, plain(0x09)), // 8
    (10, plain(0x0a)), // 9
    (11, plain(0x0b)), // 0
    (12, plain(0x0c)), // minus
    (13, plain(0x0d)), // equal
    (14, plain(0x0e)), // backspace
    (15, plain(0x0f)), // tab
    (16, plain(0x10)), // q
    (17, plain(0x11)), // w
    (18, plain(0x12)), // e
    (19, plain(0x13)), // r
    (20, plain(0x14)), // t
    (21, plain(0x15)), // y
    (22, plain(0x16)), // u
    (23, plain(0x17)), // i
    (24, plain(0x18)), // o
    (25, plain(0x19)), // p
    (26, plain(0x1a)), // leftbrace
    (27, plain(0x1b)), // rightbrace
    (28, plain(0x1c)), // enter
    (29, plain(0x1d)), // leftctrl
    (30, plain(0x1e)), // a
    (31, plain(0x1f)), // s
    (32, plain(0x20)), // d
    (33, plain(0x21)), // f
    (34, plain(0x22)), // g
    (35, plain(0x23)), // h
    (36, plain(0x24)), // j
    (37, plain(0x25)), // k
    (38, plain(0x26)), // l
    (39, plain(0x27)), // semicolon
    (40, plain(0x28)), // apostrophe
    (41, plain(0x29)), // grave
    (42, plain(0x2a)), // leftshift
    (43, plain(0x2b)), // backslash
    (44, plain(0x2c)), // z
    (45, plain(0x2d)), // x
    (46, plain(0x2e)), // c
    (47, plain(0x2f)), // v
    (48, plain(0x30)), // b
    (49, plain(0x31)), // n
    (50, plain(0x32)), // m
    (51, plain(0x33)), // comma
    (52, plain(0x34)), // dot
    (53, plain(0x35)), // slash
    (54, plain(0x36)), // rightshift
    (55, plain(0x37)), // kpasterisk
    (56, plain(0x38)), // leftalt
    (57, plain(0x39)), // space
    (58, plain(0x3a)), // capslock
    (59, plain(0x3b)), // f1
    (60, plain(0x3c)), // f2
    (61, plain(0x3d)), // f3
    (62, plain(0x3e)), // f4
    (63, plain(0x3f)), // f5
    (64, plain(0x40)), // f6
    (65, plain(0x41)), // f7
    (66, plain(0x42)), // f8
    (67, plain(0x43)), // f9
    (68, plain(0x44)), // f10
    (69, plain(0x45)), // numlock
    (70, plain(0x46)), // scrolllock
    (71, plain(0x47)), // kp7
    (72, plain(0x48)), // kp8
    (73, plain(0x49)), // kp9
    (74, plain(0x4a)), // kpminus
    (75, plain(0x4b)), // kp4
    (76, plain(0x4c)), // kp5
    (77, plain(0x4d)), // kp6
    (78, plain(0x4e)), // kpplus
    (79, plain(0x4f)), // kp1
    (80, plain(0x50)), // kp2
    (81, plain(0x51)), // kp3
    (82, plain(0x52)), // kp0
    (83, plain(0x53)), // kpdot
    (85, plain(0x76)), // zenkakuhankaku
    (86, plain(0x56)), // 102nd
    (87, plain(0x57)), // f11
    (88, plain(0x58)), // f12
    (89, plain(0x73)), // ro
    (90, plain(0x78)), // katakana
    (91, plain(0x77)), // hiragana
    (92, plain(0x79)), // henkan
    (93, plain(0x70)), // katakanahiragana
    (94, plain(0x7b)), // muhenkan
    (95, plain(0x5c)), // kpjpcomma
    (96, extended(0x1c)), // kpenter
    (97, extended(0x1d)), // rightctrl
    (98, extended(0x35)), // kpslash
    (99, extended(0x37)), // sysrq
    (100, extended(0x38)), // rightalt
    (102, extended(0x47)), // home
    (103, extended(0x48)), // up
    (104, extended(0x49)), // pageup
    (105, extended(0x4b)), // left
    (106, extended(0x4d)), // right
    (107, extended(0x4f)), // end
    (108, extended(0x50)), // down
    (109, extended(0x51)), // pagedown
    (110, extended(0x52)), // insert
    (111, extended(0x53)), // delete
    (113, extended(0x20)), // mute
    (114, extended(0x2e)), // volumedown
    (115, extended(0x30)), // volumeup
    (116, extended(0x5e)), // power
    (117, plain(0x59)), // kpequal
    (119, Code::Virtual(VK_PAUSE)), // pause: by virtual key alone
    (121, plain(0x7e)), // kpcomma
    (122, plain(0x72)), // hangeul
    (123, plain(0x71)), // hanja
    (124, plain(0x7d)), // yen
    (125, extended(0x5b)), // leftmeta
    (126, extended(0x5c)), // rightmeta
    (127, extended(0x5d)), // compose
    (128, extended(0x68)), // stop
    (140, extended(0x21)), // calc
    (142, extended(0x5f)), // sleep
    (143, extended(0x63)), // wakeup
    (144, extended(0x6b)), // file
    (155, extended(0x6c)), // mail
    (156, extended(0x66)), // bookmarks
    (158, extended(0x6a)), // back
    (159, extended(0x69)), // forward
    (163, extended(0x19)), // nextsong
    (164, extended(0x22)), // playpause
    (165, extended(0x10)), // previoussong
    (166, extended(0x24)), // stopcd
    (171, extended(0x6d)), // config
    (172, extended(0x32)), // homepage
    (173, extended(0x67)), // refresh
    (183, plain(0x64)), // f13
    (184, plain(0x65)), // f14
    (185, plain(0x66)), // f15
    (186, plain(0x67)), // f16
    (187, plain(0x68)), // f17
    (188, plain(0x69)), // f18
    (189, plain(0x6a)), // f19
    (190, plain(0x6b)), // f20
    (191, plain(0x6c)), // f21
    (192, plain(0x6d)), // f22
    (193, plain(0x6e)), // f23
    (194, plain(0x76)), // f24
    (217, extended(0x65)), // search
];

/// Codes Windows gives another key's scan code: Zenkaku/Hankaku (LANG5)
/// shares 0x76 with F24. Received, that scan code is F24, the key with a
/// name in config files.
const SHADOWED: &[u16] = &[85];

/// NumLock's code. It is sent as 0x45 without the extended flag, and Windows
/// reports it with the flag (0006 point 1), so it is read back either way.
const NUMLOCK: u16 = 69;

/// Pause's code.
const PAUSE: u16 = 119;

/// What to send Windows for `key`, or `None` for a key Windows has none for.
pub(crate) fn code(key: Key) -> Option<Code> {
    TABLE
        .binary_search_by_key(&key.code(), |&(code, _)| code)
        .ok()
        .map(|index| TABLE[index].1)
}

/// The key Windows reports as `scan`, `extended` and `virtual_key` (as a
/// window message or a hook gives them), or `None` for one the table lacks.
/// Pause is read by its virtual key, which Windows reports with scan code
/// 0x45, NumLock's. For reading keys back: hotkeys (WN08) and recording
/// (WN09).
pub fn key(scan: u8, extended: bool, virtual_key: u16) -> Option<Key> {
    let code = if virtual_key == VK_PAUSE {
        PAUSE
    } else if scan == 0x45 {
        NUMLOCK
    } else {
        let received = Code::Scan { scan, extended };
        TABLE
            .iter()
            .find(|&&(code, sent)| sent == received && !SHADOWED.contains(&code))?
            .0
    };
    Key::new(code)
}

#[cfg(test)]
mod tests;
