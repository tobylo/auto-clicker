use std::os::windows::io::AsRawHandle;

use auto_clicker_core::input::{Held, Key};
use windows_sys::Win32::Foundation::GetHandleInformation;
use windows_sys::Win32::System::Threading::TerminateProcess;

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
    // A child that lost its test name runs every test, this one included:
    // it starts no child of its own.
    if argument("table").is_some() {
        return;
    }
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
    assert_eq!(exit_code(&daemon), 0, "the child's checks failed");
    // The child marked a key of its own in the table it was given.
    assert!(section.table().owes(key(30)));
    drop(stray);
}

/// How the child `daemon` ended, waiting a minute at most: past that it is
/// killed and the test fails.
#[allow(unsafe_code)]
fn exit_code(daemon: &Daemon) -> u32 {
    let process = daemon.process.as_raw_handle();
    // SAFETY: a live process handle with full access.
    if unsafe { WaitForSingleObject(process, 60_000) } != WAIT_OBJECT_0 {
        // SAFETY: as above.
        unsafe { TerminateProcess(process, 1) };
        panic!("the child did not end within a minute");
    }
    let mut code = 0u32;
    // SAFETY: as above; `code` is writable.
    assert_ne!(unsafe { GetExitCodeProcess(process, &mut code) }, 0);
    code
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
    // A real handle that is no process: only the process check refuses it.
    let second = Section::create().unwrap();
    let other = second.raw() as usize;
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
            "no handle for the watchdog",
            Handles {
                table,
                watchdog: 0x7ff0,
            },
        ),
        (
            "a section for the watchdog",
            Handles {
                table,
                watchdog: other,
            },
        ),
    ] {
        assert!(attach(handles).is_err(), "{name}");
    }
    // All are still open: a refusal closes nothing.
    assert!(flags(table).is_some() && flags(watchdog).is_some() && flags(other).is_some());
}
