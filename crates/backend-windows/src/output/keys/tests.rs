use std::collections::HashMap;

use super::*;

fn named_keys() -> Vec<Key> {
    Key::CODES
        .filter_map(Key::new)
        .filter(|key| key.name().is_some())
        .collect()
}

fn named(name: &str) -> Key {
    Key::from_name(name).unwrap()
}

/// What Windows reports for a key sent as `code`: a scan code comes back as
/// itself with no virtual key yet, and Pause as its virtual key with
/// NumLock's scan code.
fn received(code: Code) -> (u8, bool, u16) {
    match code {
        Code::Scan { scan, extended } => (scan, extended, 0),
        Code::Virtual(virtual_key) => (0x45, false, virtual_key),
    }
}

#[test]
fn the_table_is_in_code_order_with_each_code_once() {
    assert!(TABLE.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(TABLE.iter().all(|&(code, _)| Key::new(code).is_some()));
}

#[test]
fn every_named_key_has_a_windows_key_and_reads_back_as_itself() {
    let keys = named_keys();
    assert_eq!(keys.len(), 117, "config's key names changed");
    for each in keys {
        let name = each.name();
        let sent = code(each).unwrap_or_else(|| panic!("{name:?} has no Windows key"));
        let (scan, extended, virtual_key) = received(sent);
        assert_eq!(key(scan, extended, virtual_key), Some(each), "{name:?}");
    }
}

#[test]
fn every_key_in_the_table_reads_back_as_itself_unless_shadowed() {
    for &(linux, sent) in TABLE {
        let (scan, extended, virtual_key) = received(sent);
        let back = key(scan, extended, virtual_key).map(Key::code);
        if SHADOWED.contains(&linux) {
            assert_ne!(back, Some(linux), "{linux} is listed as shadowed");
        } else {
            assert_eq!(back, Some(linux));
        }
    }
}

#[test]
fn only_the_shadowed_keys_share_a_scan_code() {
    let mut sharing: HashMap<Code, Vec<u16>> = HashMap::new();
    for &(linux, sent) in TABLE {
        sharing.entry(sent).or_default().push(linux);
    }
    let mut shared: Vec<Vec<u16>> = sharing
        .into_values()
        .filter(|codes| codes.len() > 1)
        .collect();
    shared.sort();
    // Zenkaku/Hankaku (LANG5) and F24.
    assert_eq!(shared, vec![vec![85, 194]]);
}

#[test]
fn keys_go_as_x03_measured() {
    let cases = [
        ("numlock", plain(0x45)),
        ("pause", Code::Virtual(VK_PAUSE)),
        ("sysrq", extended(0x37)),
        ("f13", plain(0x64)),
        ("f24", plain(0x76)),
        ("rightctrl", extended(0x1d)),
        ("rightalt", extended(0x38)),
        ("kpenter", extended(0x1c)),
        ("kpslash", extended(0x35)),
        ("kpasterisk", plain(0x37)),
        ("up", extended(0x48)),
        ("kp8", plain(0x48)),
        ("leftmeta", extended(0x5b)),
        ("rightmeta", extended(0x5c)),
        ("compose", extended(0x5d)),
        ("102nd", plain(0x56)),
        ("backslash", plain(0x2b)),
    ];
    for (name, expected) in cases {
        assert_eq!(code(named(name)), Some(expected), "{name}");
    }
}

#[test]
fn numlock_reads_back_with_or_without_the_extended_flag() {
    assert_eq!(key(0x45, false, 0), Some(named("numlock")));
    assert_eq!(key(0x45, true, 0), Some(named("numlock")));
}

#[test]
fn a_key_windows_lacks_has_no_code() {
    // KEY_MACRO and KEY_PROG1: recordable on Linux, no key on Windows.
    for code_without in [112, 148] {
        assert_eq!(code(Key::new(code_without).unwrap()), None);
    }
    assert_eq!(key(0x00, false, 0), None);
}
