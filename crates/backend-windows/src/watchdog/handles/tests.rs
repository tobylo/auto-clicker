use std::os::windows::io::AsRawHandle;

use auto_clicker_core::input::{Held, Key};
use windows_sys::Win32::Foundation::GetHandleInformation;

use super::*;

/// Set as an argument of the child the inheritance test starts.
const CHILD: &str = "watchdog::handles::tests::the_child_side_of_inheritance";

fn key(code: u16) -> Held {
    Held::Key(Key::new(code).unwrap())
}

fn split(line: &[u16]) -> String {
    String::from_utf16(&line[..line.len() - 1]).unwrap()
}

#[test]
fn the_command_line_quotes_what_needs_it_as_the_c_runtime_reads_it() {
    let arguments: Vec<OsString> = [
        "--table",
        "1234",
        "a b",
        "",
        r#"say "hi""#,
        r"C:\dir\",
        r"C:\a b\",
        r"x\\y",
        r#"a\"b c"#,
    ]
    .iter()
    .map(OsString::from)
    .collect();
    // Backslashes count only before a quote: doubled there, kept elsewhere.
    assert_eq!(
        split(&command_line(
            OsStr::new(r"C:\Program Files\a.exe"),
            &arguments
        )),
        r#""C:\Program Files\a.exe" --table 1234 "a b" "" "say \"hi\"" C:\dir\ "C:\a b\\" x\\y "a\\\"b c""#
    );
}

/// A handle's flags; `None` when the value names no handle here.
#[allow(unsafe_code)]
fn flags(handle: usize) -> Option<u32> {
    let mut flags = 0u32;
    // SAFETY: any value may be asked; one that names no handle fails.
    (unsafe { GetHandleInformation(handle as HANDLE, &mut flags) } != 0).then_some(flags)
}

#[test]
fn a_child_inherits_the_table_and_the_watchdog_and_nothing_else() {
    let section = Section::create().unwrap();
    section.table().mark(key(17));
    // Inheritable, but not in the list: the child must not have it. A
    // section with a mark of its own, since the child may hold a handle of
    // its own with the same value.
    let stray = Section::create().unwrap();
    stray.table().mark(key(99));
    let stray_value = stray.raw() as usize;
    let daemon = start_daemon(&std::env::current_exe().unwrap(), &section, |handles| {
        [
            "--exact".to_owned(),
            CHILD.to_owned(),
            "--ignored".to_owned(),
            "--test-threads=1".to_owned(),
            format!("table={}", handles.table),
            format!("watchdog={}", handles.watchdog),
            format!("stray={stray_value}"),
        ]
        .map(OsString::from)
        .to_vec()
    })
    .unwrap();
    assert_eq!(daemon.wait().unwrap(), 0, "the child's checks failed");
    // The child marked a key of its own in the table it was given.
    assert!(section.table().owes(key(30)));
    drop(stray);
}

/// The value of `name=` among the arguments.
fn argument(name: &str) -> Option<usize> {
    std::env::args().find_map(|argument| argument.strip_prefix(&format!("{name}="))?.parse().ok())
}

#[test]
#[ignore = "the child the inheritance test starts; does nothing on its own"]
fn the_child_side_of_inheritance() {
    let (Some(table), Some(watchdog), Some(stray)) =
        (argument("table"), argument("watchdog"), argument("stray"))
    else {
        return;
    };
    assert!(
        map(stray as HANDLE).map_or(true, |table| !table.owes(key(99))),
        "a handle outside the list was inherited"
    );
    assert_eq!(
        flags(watchdog).map(|f| f & HANDLE_FLAG_INHERIT),
        Some(HANDLE_FLAG_INHERIT)
    );
    let attached = attach(Handles { table, watchdog }).unwrap();
    assert!(attached.table.owes(key(17)));
    attached.table.mark(key(30));
    // Nothing the daemon starts inherits either: the section's handle is
    // closed, and the watchdog's no longer inheritable.
    assert_eq!(flags(table), None);
    assert_eq!(flags(watchdog).map(|f| f & HANDLE_FLAG_INHERIT), Some(0));
}

#[test]
fn handles_that_are_not_what_they_should_be_are_refused_and_left_open() {
    let section = Section::create().unwrap();
    let process = own_process().unwrap();
    let table = section.raw() as usize;
    let watchdog = process.as_raw_handle() as usize;
    for (name, handles) in [
        (
            "the same handle twice",
            Handles {
                table,
                watchdog: table,
            },
        ),
        (
            "swapped",
            Handles {
                table: watchdog,
                watchdog: table,
            },
        ),
        (
            "no handle at all",
            Handles {
                table: 0x7ff0,
                watchdog,
            },
        ),
        (
            "a section for the watchdog",
            Handles {
                table,
                watchdog: 0x7ff0,
            },
        ),
    ] {
        assert!(attach(handles).is_err(), "{name}");
    }
    // Both are still open: a refusal closes nothing.
    assert!(flags(table).is_some() && flags(watchdog).is_some());
}
