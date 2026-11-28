use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use windows_sys::Win32::Foundation::GetHandleInformation;
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};

use super::*;

/// Set as the test name of the child the pipe test starts.
const CHILD: &str = "settings_window::tests::the_window_side_of_the_pipes";
/// What the child sends on its requests pipe, and what it prints.
const SENT: &str = "{\"sent\":\"on the pipe\"}";
const PRINTED: &str = "{\"printed\":\"on standard output\"}";

fn args(words: &[&str]) -> impl Iterator<Item = OsString> {
    words
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>()
        .into_iter()
}

#[test]
fn the_pipes_are_taken_from_the_command_line_and_only_once() {
    // The test's own command line names no pipes; a second try is refused
    // before it reads one.
    let first = take_inherited().unwrap_err();
    assert_eq!(first.kind(), io::ErrorKind::InvalidInput, "{first}");
    let second = take_inherited().unwrap_err();
    assert_ne!(second.kind(), io::ErrorKind::InvalidInput, "{second}");
}

#[test]
fn the_pipes_are_read_back_in_the_form_they_are_written_only() {
    let pipes = Pipes {
        requests: 0x1a4,
        updates: 0x1b0,
    };
    assert_eq!(parse(arguments(pipes).into_iter()), Ok(pipes));
    for refused in [
        &[][..],
        &["--requests", "420"],
        &["--requests", "420", "--updates"],
        &["--updates", "432", "--requests", "420"],
        &["--requests", "420", "--updates", "420"],
        &["--requests", "0", "--updates", "432"],
        &["--requests", "0420", "--updates", "432"],
        &["--requests", "+420", "--updates", "432"],
        &["--requests", "-420", "--updates", "432"],
        &["--requests", "0x1a4", "--updates", "432"],
        // A handle's low two bits are Windows' own: 421 is 420.
        &["--requests", "421", "--updates", "432"],
        &["--requests", "420", "--updates", "422"],
        &["--requests", "420", "--updates", "432", "more"],
    ] {
        assert!(parse(args(refused)).is_err(), "{refused:?}");
    }
}

/// A handle's flags; `None` when the value names no handle here.
#[allow(unsafe_code)]
fn flags(handle: usize) -> Option<u32> {
    let mut flags = 0u32;
    // SAFETY: any value may be asked; one that names no handle fails.
    (unsafe { GetHandleInformation(handle as HANDLE, &mut flags) } != 0).then_some(flags)
}

#[test]
fn a_handle_that_is_no_pipe_is_refused_and_left_open() {
    let file = File::open(std::env::current_exe().unwrap()).unwrap();
    let (read, write) = io::pipe().unwrap();
    let file_value = file.as_raw_handle() as usize;
    let write = OwnedHandle::from(write);
    let pipes = Pipes {
        requests: write.as_raw_handle() as usize,
        updates: file_value,
    };
    assert!(take(pipes).is_err());
    assert!(flags(file_value).is_some(), "the file was closed");
    drop((file, read, write));
}

#[test]
fn taken_pipes_are_kept_from_children() {
    let (read, write) = io::pipe().unwrap();
    let (read, write) = (OwnedHandle::from(read), OwnedHandle::from(write));
    for handle in [&read, &write] {
        // SAFETY: a handle of this test's own.
        #[allow(unsafe_code)]
        let set = unsafe {
            SetHandleInformation(
                handle.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        };
        assert_ne!(set, 0);
        let flags = flags(handle.as_raw_handle() as usize).unwrap();
        assert_eq!(flags & HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
    }
    let pipes = Pipes {
        requests: write.as_raw_handle() as usize,
        updates: read.as_raw_handle() as usize,
    };
    // Owned by `take` from here on.
    std::mem::forget((read, write));
    let (requests, updates) = take(pipes).unwrap();
    for file in [&requests, &updates] {
        let flags = flags(file.as_raw_handle() as usize).unwrap();
        assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
    }
}

/// The child's side of the pipe test: what the window does with what it
/// inherits. It prints a line of the protocol on its standard output and
/// error, which it has none of, sends one on its requests pipe, and echoes
/// the first update back.
#[test]
fn the_window_side_of_the_pipes() {
    let given: Vec<OsString> = std::env::args_os().collect();
    let Some(at) = given.iter().position(|arg| arg == REQUESTS) else {
        return;
    };
    // SAFETY: plain calls.
    #[allow(unsafe_code)]
    let standard = unsafe {
        [
            GetStdHandle(STD_OUTPUT_HANDLE),
            GetStdHandle(STD_ERROR_HANDLE),
        ]
    };
    assert!(standard.iter().all(|handle| handle.is_null()));
    // Straight to the standard streams, as a library prints: the test
    // harness captures `println!`, and would keep it from them.
    let _ = writeln!(io::stdout(), "{PRINTED}");
    let _ = writeln!(io::stderr(), "{PRINTED}");
    let pipes = parse(given[at..].iter().cloned()).unwrap();
    let (mut requests, updates) = take(pipes).unwrap();
    writeln!(requests, "{SENT}").unwrap();
    let mut update = String::new();
    BufReader::new(updates).read_line(&mut update).unwrap();
    write!(requests, "{update}").unwrap();
}

/// Starts this test program as the window, running `test` alone.
fn start_child(test: &str) -> Started {
    let before = [test, "--exact", "--"].map(OsString::from);
    start_with(&std::env::current_exe().unwrap(), &before).unwrap()
}

#[tokio::test]
async fn only_what_the_window_sends_on_its_pipe_reaches_the_daemon() {
    let Started {
        window,
        requests,
        mut updates,
    } = start_child(CHILD);
    let mut requests = tokio::io::BufReader::new(requests);
    let mut line = String::new();
    let read = tokio::time::timeout(Duration::from_secs(20), requests.read_line(&mut line));
    read.await.unwrap().unwrap();
    assert_eq!(line.trim_end(), SENT);
    updates.write_all(b"{\"update\":1}\n").await.unwrap();
    line.clear();
    let read = tokio::time::timeout(Duration::from_secs(20), requests.read_line(&mut line));
    read.await.unwrap().unwrap();
    assert_eq!(line.trim_end(), "{\"update\":1}");
    // The window exits; nothing else ever came on the pipe.
    line.clear();
    let read = tokio::time::timeout(Duration::from_secs(20), requests.read_line(&mut line));
    assert_eq!(read.await.unwrap().unwrap(), 0, "{line}");
    drop(window);
}

/// A child that says it is ready, then reads its updates until they end,
/// then exits.
#[test]
fn a_window_that_waits_for_its_updates() {
    let given: Vec<OsString> = std::env::args_os().collect();
    let Some(at) = given.iter().position(|arg| arg == REQUESTS) else {
        return;
    };
    let (mut requests, updates) = take(parse(given[at..].iter().cloned()).unwrap()).unwrap();
    writeln!(requests, "{SENT}").unwrap();
    let _ = BufReader::new(updates).lines().count();
}

#[tokio::test]
async fn a_dropped_window_is_killed_and_its_pipes_end() {
    let Started {
        window,
        requests,
        updates,
    } = start_child("settings_window::tests::a_window_that_waits_for_its_updates");
    let mut requests = tokio::io::BufReader::new(requests);
    // The window runs, and holds its pipes.
    let mut line = String::new();
    let read = tokio::time::timeout(Duration::from_secs(20), requests.read_line(&mut line));
    read.await.unwrap().unwrap();
    assert_eq!(line.trim_end(), SENT);
    drop(window);
    // The window was killed: its end of the requests pipe closed, though
    // the daemon still holds the updates pipe it was waiting on.
    line.clear();
    let read = tokio::time::timeout(Duration::from_secs(5), requests.read_line(&mut line));
    assert_eq!(read.await.unwrap().unwrap(), 0, "{line}");
    drop(updates);
}

#[test]
fn a_pipe_another_program_connected_to_first_starts_no_window() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .unwrap();
    let _inside = runtime.enter();
    let name = pipe_name();
    let _server = server(&name, Way::ToWindow).unwrap();
    // Another program got there first: the window's end cannot be opened.
    let _first = window_end(&name, Way::ToWindow).unwrap();
    assert!(window_end(&name, Way::ToWindow).is_err());
    // Nor can a second pipe of the same name be made.
    assert!(server(&name, Way::ToWindow).is_err());
    assert_ne!(pipe_name(), pipe_name());
}

/// How far the daemon's end of a pipe may act as the window that opened
/// the other: the impersonation level its token gets.
#[allow(unsafe_code)]
fn impersonation_level(server: &NamedPipeServer) -> i32 {
    use windows_sys::Win32::Security::{
        GetTokenInformation, RevertToSelf, TOKEN_QUERY, TokenImpersonationLevel,
    };
    use windows_sys::Win32::System::Pipes::ImpersonateNamedPipeClient;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};
    // SAFETY: the pipe is this test's, with a byte read from it, as
    // impersonation requires.
    assert_ne!(
        unsafe { ImpersonateNamedPipeClient(server.as_raw_handle()) },
        0
    );
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the thread impersonates now; `token` is live.
    let opened = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) };
    // SAFETY: plain call; the thread goes back to its own token.
    assert_ne!(unsafe { RevertToSelf() }, 0);
    assert_ne!(opened, 0, "{}", io::Error::last_os_error());
    // SAFETY: a new token handle, owned from here on.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut level = 0i32;
    let mut size = 0u32;
    // SAFETY: `level` is live and as large as the size passed.
    let read = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenImpersonationLevel,
            (&raw mut level).cast(),
            u32::try_from(size_of::<i32>()).unwrap(),
            &mut size,
        )
    };
    assert_ne!(read, 0, "{}", io::Error::last_os_error());
    level
}

#[tokio::test]
async fn the_daemon_may_identify_the_window_but_never_act_as_it() {
    use tokio::io::AsyncReadExt;
    use windows_sys::Win32::Security::SecurityIdentification;
    let name = pipe_name();
    let mut server = server(&name, Way::FromWindow).unwrap();
    let theirs = window_end(&name, Way::FromWindow).unwrap();
    connected(&server).unwrap();
    let mut theirs = File::from(theirs);
    theirs.write_all(b"x").unwrap();
    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(5), server.read_exact(&mut byte));
    read.await.unwrap().unwrap();
    assert_eq!(impersonation_level(&server), SecurityIdentification);
}
