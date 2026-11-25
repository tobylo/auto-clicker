use std::time::Instant;

use windows_sys::Win32::System::RemoteDesktop::WTSSessionInfoEx;
use windows_sys::Win32::UI::WindowsAndMessaging::WTS_SESSION_LOGON;

use super::*;

/// Long enough for any event to arrive on a slow runner.
const ARRIVAL: Duration = Duration::from_secs(10);
/// How long a quit may take: far above what it does, far below forever.
const JOIN: Duration = Duration::from_secs(5);

/// A message thread whose events arrive on a channel.
fn started() -> (MessageThread, mpsc::Receiver<WindowEvent>) {
    let (events, received) = mpsc::channel();
    let thread = MessageThread::spawn(
        move |event| {
            let _ = events.send(event);
        },
        ARRIVAL,
    )
    .unwrap();
    (thread, received)
}

/// What a test follows of an event; `None` for what Windows sends on its
/// own (the session's and the display's state, other messages), which
/// differs between a runner and a desktop.
fn followed(event: &WindowEvent) -> Option<String> {
    match event {
        WindowEvent::Session(change) => Some(format!("{change:?}")),
        WindowEvent::Suspend => Some("Suspend".to_owned()),
        WindowEvent::Resume => Some("Resume".to_owned()),
        WindowEvent::EndSession => Some("EndSession".to_owned()),
        WindowEvent::Close => Some("Close".to_owned()),
        WindowEvent::Ended { failure: None } => Some("Ended".to_owned()),
        WindowEvent::Ended { failure: Some(why) } => Some(format!("Failed: {why}")),
        WindowEvent::SessionRead(_)
        | WindowEvent::SessionUnwatched(_)
        | WindowEvent::Display(_)
        | WindowEvent::DisplayUnwatched(_)
        | WindowEvent::Other(_) => None,
    }
}

/// The next `count` followed events, in order.
fn next_followed(received: &mpsc::Receiver<WindowEvent>, count: usize) -> Vec<String> {
    let deadline = Instant::now() + ARRIVAL;
    let mut seen = Vec::new();
    while seen.len() < count {
        let left = deadline.saturating_duration_since(Instant::now());
        match received.recv_timeout(left) {
            Ok(event) => seen.extend(followed(&event)),
            Err(err) => panic!("after {seen:?}: {err}"),
        }
    }
    seen
}

fn session_code(code: u32) -> WPARAM {
    usize::try_from(code).unwrap()
}

#[test]
fn notices_reach_the_handler_in_order_and_quit_ends_the_thread() {
    let (thread, received) = started();
    let remote = thread.remote();
    let posts = [
        (WM_WTSSESSION_CHANGE, session_code(WTS_SESSION_LOCK)),
        // Not followed: it comes to nothing.
        (WM_WTSSESSION_CHANGE, session_code(WTS_SESSION_LOGON)),
        (WM_WTSSESSION_CHANGE, session_code(WTS_CONSOLE_DISCONNECT)),
        (WM_WTSSESSION_CHANGE, session_code(WTS_REMOTE_CONNECT)),
        (WM_WTSSESSION_CHANGE, session_code(WTS_SESSION_UNLOCK)),
        (WM_POWERBROADCAST, session_code(PBT_APMSUSPEND)),
        (WM_POWERBROADCAST, session_code(PBT_APMRESUMEAUTOMATIC)),
        (WM_POWERBROADCAST, session_code(PBT_APMRESUMESUSPEND)),
        // A logoff that was called off: nothing ends.
        (WM_ENDSESSION, 0),
        (WM_ENDSESSION, 1),
        (WM_CLOSE, 0),
        // A close leaves the window: it still hears notices.
        (WM_WTSSESSION_CHANGE, session_code(WTS_SESSION_LOCK)),
    ];
    for (message, wparam) in posts {
        remote.post(message, wparam, 0).unwrap();
    }
    assert_eq!(
        next_followed(&received, 10),
        [
            "Lock",
            "Disconnect",
            "Connect",
            "Unlock",
            "Suspend",
            "Resume",
            "Resume",
            "EndSession",
            "Close",
            "Lock"
        ]
    );
    let quitting = Instant::now();
    thread.quit_and_join(JOIN).unwrap();
    assert!(quitting.elapsed() < JOIN, "{:?}", quitting.elapsed());
    assert_eq!(next_followed(&received, 1), ["Ended"]);
    // Nothing comes after the last event.
    assert!(received.recv_timeout(Duration::from_millis(100)).is_err());
}

#[test]
fn once_the_thread_ended_nothing_posts_to_its_window() {
    let (thread, _received) = started();
    let remote = thread.remote();
    thread.quit_and_join(JOIN).unwrap();
    let err = remote.quit().unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotConnected);
}

#[test]
fn a_panicking_handler_ends_the_thread_and_says_why() {
    let (events, received) = mpsc::channel();
    let thread = MessageThread::spawn(
        move |event| {
            if matches!(event, WindowEvent::Close) {
                panic!("on close");
            }
            let _ = events.send(event);
        },
        ARRIVAL,
    )
    .unwrap();
    thread.remote().post(WM_CLOSE, 0, 0).unwrap();
    let ended = next_followed(&received, 1);
    assert!(
        ended[0].starts_with("Failed: the message thread panicked: on close"),
        "{ended:?}"
    );
    // The thread destroyed its window on the way out and ended.
    thread.quit_and_join(JOIN).unwrap();
}

#[test]
fn the_join_gives_up_at_its_deadline() {
    let (events, received) = mpsc::channel();
    let thread = MessageThread::spawn(
        move |event| {
            if matches!(event, WindowEvent::EndSession) {
                // A handler stuck far longer than the join waits.
                thread::sleep(Duration::from_secs(3));
            }
            let _ = events.send(event);
        },
        ARRIVAL,
    )
    .unwrap();
    thread.remote().post(WM_ENDSESSION, 1, 0).unwrap();
    // Let the handler start on it.
    thread::sleep(Duration::from_millis(100));
    let joining = Instant::now();
    let joined = thread.quit_and_join(Duration::from_millis(200));
    assert!(matches!(joined, Err(JoinError::TimedOut)), "{joined:?}");
    assert!(
        joining.elapsed() < Duration::from_secs(2),
        "{:?}",
        joining.elapsed()
    );
    // The thread still ends once its handler is done.
    assert_eq!(next_followed(&received, 2), ["EndSession", "Ended"]);
}

#[test]
fn session_notices_map_to_changes() {
    let cases = [
        (WTS_SESSION_LOCK, Some(SessionChange::Lock)),
        (WTS_SESSION_UNLOCK, Some(SessionChange::Unlock)),
        (WTS_CONSOLE_CONNECT, Some(SessionChange::Connect)),
        (WTS_REMOTE_CONNECT, Some(SessionChange::Connect)),
        (WTS_CONSOLE_DISCONNECT, Some(SessionChange::Disconnect)),
        (WTS_REMOTE_DISCONNECT, Some(SessionChange::Disconnect)),
        (WTS_SESSION_LOGON, None),
        (u32::try_from(WTSSessionInfoEx).unwrap(), None),
    ];
    for (code, change) in cases {
        assert_eq!(
            SessionChange::from_code(session_code(code)),
            change,
            "{code}"
        );
    }
    assert_eq!(SessionChange::from_code(usize::MAX), None);
}

/// A `POWERBROADCAST_SETTING` with four bytes of data.
#[repr(C)]
struct Setting {
    header: POWERBROADCAST_SETTING,
    rest: [u8; 3],
}

impl Setting {
    fn new(guid: windows_sys::core::GUID, length: u32, value: u32) -> Setting {
        let [first, rest @ ..] = value.to_le_bytes();
        Setting {
            header: POWERBROADCAST_SETTING {
                PowerSetting: guid,
                DataLength: length,
                Data: [first],
            },
            rest,
        }
    }

    fn read(&self) -> Option<bool> {
        display_setting(std::ptr::from_ref(self) as LPARAM)
    }
}

#[test]
fn the_display_setting_says_on_or_off() {
    let display = GUID_CONSOLE_DISPLAY_STATE;
    assert_eq!(Setting::new(display, 4, 0).read(), Some(false));
    assert_eq!(Setting::new(display, 4, 1).read(), Some(true));
    // Dimmed is on.
    assert_eq!(Setting::new(display, 4, 2).read(), Some(true));
    // Too short, or another setting: nothing.
    assert_eq!(Setting::new(display, 1, 0).read(), None);
    let mut other = display;
    other.data1 ^= 1;
    assert_eq!(Setting::new(other, 4, 0).read(), None);
    assert_eq!(display_setting(0), None);
}
