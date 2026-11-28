//! The tray's tests. Those that need the taskbar and real input are
//! ignored, and run one at a time in the WN01 VM's desktop:
//!
//! ```text
//! scripts/windows-vm/vm run --timeout 600 --workdir 'C:\work\<dir>' -- cargo test --locked -p auto-clicker-backend-windows tray -- --ignored --test-threads=1 --nocapture
//! ```

mod desktop;

use std::sync::mpsc;
use std::time::Instant;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    WM_ACTIVATEAPP, WM_CLOSE, WM_ENTERIDLE, WM_KEYUP, WM_TIMER,
};

use super::*;
use crate::message_thread::tests::{ARRIVAL, started};

/// Every event up to the next `Close`, which ends what a test posted.
fn until_close(received: &mpsc::Receiver<WindowEvent>) -> Vec<WindowEvent> {
    let deadline = Instant::now() + ARRIVAL;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match received.recv_timeout(left) {
            Ok(WindowEvent::Close) => return seen,
            Ok(event) => seen.push(event),
            Err(err) => panic!("after {seen:?}: {err}"),
        }
    }
}

#[test]
fn what_the_users_hand_moves_is_no_wake() {
    for message in [
        WM_NULL,
        WM_ACTIVATE,
        WM_ACTIVATEAPP,
        WM_NCACTIVATE,
        WM_KEYUP,
    ] {
        assert!(users_hand(message), "{message:#06x}");
    }
    for message in [WM_TIMER, WM_ENTERIDLE, WM_TRAY_REFRESH, 0x0219] {
        assert!(!users_hand(message), "{message:#06x}");
    }
}

#[test]
fn the_menus_anchor_is_two_signed_words() {
    // x = -5, y = 1056: a monitor left of the main one.
    let wparam = (1056 << 16) | 0xfffb;
    assert_eq!(anchor(wparam), (-5, 1056));
}

#[test]
fn the_front_and_the_keys_wake_nothing() {
    let (thread, received) = started();
    let remote = thread.remote();
    for (message, wparam) in [
        (WM_NULL, 0),
        (WM_ACTIVATEAPP, 0),
        (WM_KEYUP, 0x1b),
        (WM_CLOSE, 0),
    ] {
        remote.post(message, wparam, 0).unwrap();
    }
    let seen = until_close(&received);
    // Windows says things of its own as the window comes up; none of what
    // the test posted is among them.
    assert!(
        !seen.iter().any(|event| matches!(
            event,
            WindowEvent::Other(WM_NULL | WM_ACTIVATEAPP | WM_KEYUP)
                | WindowEvent::TrayUnshown(_)
                | WindowEvent::TrayShown
        )),
        "{seen:?}"
    );
    thread.quit_and_join(ARRIVAL).unwrap();
}

/// Content with one of each kind of entry.
pub fn content(title: &str) -> TrayContent {
    let item = |label: &str, command, enabled| MenuEntry::Item {
        label: label.to_owned(),
        command,
        enabled,
        checked: None,
    };
    TrayContent {
        title: title.to_owned(),
        details: vec!["A first line".to_owned(), "Rock & roll".to_owned()],
        icon: TrayIcon {
            side: icon_side(),
            rgba: [0x43, 0x38, 0xca, 0xff].repeat(usize::from(icon_side()).pow(2)),
        },
        menu: vec![
            item("Start clicking", Some(1), true),
            MenuEntry::Submenu {
                label: "Play".to_owned(),
                children: vec![MenuEntry::Item {
                    label: "Rock & roll".to_owned(),
                    command: Some(100),
                    enabled: true,
                    checked: Some(true),
                }],
            },
            item("Record", None, false),
            MenuEntry::Separator,
            item("Quit", Some(7), true),
        ],
    }
}

#[test]
fn without_explorers_restart_notice_a_refused_icon_stays_on_the_ladder() {
    assert!(on_ladder(true, || true));
    assert!(!on_ladder(true, || false));
    // Waiting for a notice that cannot be heard would wait for ever.
    assert!(on_ladder(false, || false));
}

#[test]
fn a_picture_the_taskbar_cannot_take_reaches_the_handler_as_refused() {
    let (thread, received) = started();
    let mut broken = content("Auto clicker: idle");
    broken.icon = TrayIcon {
        side: 2,
        rgba: vec![0; 3],
    };
    thread.remote().show_tray(broken).unwrap();
    let deadline = Instant::now() + ARRIVAL;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match received.recv_timeout(left) {
            Ok(WindowEvent::TrayUnshown(err)) => {
                assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
                break;
            }
            Ok(_) => {}
            Err(err) => panic!("no refusal: {err}"),
        }
    }
    thread.quit_and_join(ARRIVAL).unwrap();
}

#[test]
#[allow(unsafe_code)]
fn explorers_restart_is_heard() {
    let (thread, received) = started();
    let name = wide("TaskbarCreated");
    // SAFETY: `name` is NUL-terminated UTF-16, alive for the call; the
    // number is the same in every thread.
    let taskbar_created = unsafe { RegisterWindowMessageW(name.as_ptr()) };
    let remote = thread.remote();
    remote.post(taskbar_created, 0, 0).unwrap();
    remote.post(WM_CLOSE, 0, 0).unwrap();
    let seen = until_close(&received);
    assert!(
        seen.iter()
            .any(|event| matches!(event, WindowEvent::TrayRestarted)),
        "{seen:?}"
    );
    assert!(
        !seen
            .iter()
            .any(|event| matches!(event, WindowEvent::Other(n) if *n == taskbar_created)),
        "{seen:?}"
    );
    thread.quit_and_join(ARRIVAL).unwrap();
}
