//! The hotkeys' registration on a real message thread. Raw input is
//! registered per process and hotkeys for the whole session, so the tests
//! take turns, and use combinations no one else does.

use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use windows_sys::Win32::UI::Input::GetRegisteredRawInputDevices;

use super::*;
use crate::message_thread::tests::{ARRIVAL, JOIN, started};

/// One test at a time: what they register is shared.
static TURN: Mutex<()> = Mutex::new(());

fn binding(text: &str) -> Binding {
    Binding::parse(text).unwrap()
}

/// Bindings nothing else on a test machine holds.
fn rare() -> Bindings {
    Bindings {
        toggle_clicking: binding("Ctrl+Alt+Shift+F21"),
        toggle_recording: binding("Ctrl+Alt+Shift+F22"),
        toggle_playback: binding("Ctrl+Alt+Shift+F23"),
    }
}

/// A sink whose events arrive on a channel.
fn sink() -> (Sink, mpsc::Receiver<HotkeyEvent>) {
    let (events, received) = mpsc::channel();
    let events = Mutex::new(events);
    let sink: Sink = Arc::new(move |event| {
        let _ = events.lock().unwrap().send(event);
    });
    (sink, received)
}

fn next(received: &mpsc::Receiver<HotkeyEvent>) -> HotkeyEvent {
    received.recv_timeout(ARRIVAL).unwrap()
}

/// How many kinds of raw input this process reads.
#[allow(unsafe_code)]
fn raw_registrations() -> u32 {
    let mut count = 0u32;
    let size = u32::try_from(size_of::<RAWINPUTDEVICE>()).unwrap();
    // SAFETY: a null buffer asks for the count alone.
    unsafe { GetRegisteredRawInputDevices(null_mut(), &mut count, size) };
    count
}

/// Waits until this process reads `count` kinds of raw input.
fn reads(count: u32) {
    let deadline = Instant::now() + ARRIVAL;
    while raw_registrations() != count {
        assert!(Instant::now() < deadline, "still {}", raw_registrations());
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Whether `binding` is free: this thread can register it, and does not
/// keep it.
#[allow(unsafe_code)]
fn free(binding: Binding) -> bool {
    let vk = virtual_key(binding.key()).unwrap();
    let modifiers = MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT;
    // SAFETY: a thread hotkey (no window), removed again at once.
    unsafe {
        let took = RegisterHotKey(null_mut(), 0xB00, modifiers, u32::from(vk)) != 0;
        if took {
            UnregisterHotKey(null_mut(), 0xB00);
        }
        took
    }
}

#[test]
fn the_keyboard_is_read_only_while_asked_and_the_hotkeys_go_with_their_reader() {
    let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let (thread, _) = started();
    let (sink, received) = sink();
    let reading = thread.remote().read_hotkeys(rare(), sink).unwrap();
    assert_eq!(
        next(&received),
        HotkeyEvent::Triggers(Triggers::bound(&rare()))
    );
    assert_eq!(
        next(&received),
        HotkeyEvent::Status(HotkeyStatus::Available)
    );
    assert!(!free(rare().toggle_clicking), "registered");
    // Idle: no raw input.
    assert_eq!(raw_registrations(), 0);
    let stops = Watching {
        release: false,
        stops: vec![Action::ToggleClicking],
    };
    reading.watch(stops).unwrap();
    reads(1);
    reading.watch(Watching::default()).unwrap();
    reads(0);
    drop(reading);
    let deadline = Instant::now() + ARRIVAL;
    while !free(rare().toggle_clicking) {
        assert!(Instant::now() < deadline, "the hotkey stayed registered");
        std::thread::sleep(Duration::from_millis(5));
    }
    thread.quit_and_join(JOIN).unwrap();
}

#[test]
fn a_replaced_readers_end_leaves_its_replacements_hotkeys() {
    let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let (thread, _) = started();
    let (first_sink, _first) = sink();
    let first = thread.remote().read_hotkeys(rare(), first_sink).unwrap();
    let (second_sink, second) = sink();
    let replacement = thread.remote().read_hotkeys(rare(), second_sink).unwrap();
    drop(first);
    assert_eq!(
        next(&second),
        HotkeyEvent::Triggers(Triggers::bound(&rare()))
    );
    assert_eq!(next(&second), HotkeyEvent::Status(HotkeyStatus::Available));
    // Raw input read for the replacement shows the thread got past the
    // first reader's end: its hotkeys stayed.
    let stops = Watching {
        release: false,
        stops: vec![Action::ToggleClicking],
    };
    replacement.watch(stops).unwrap();
    reads(1);
    assert!(!free(rare().toggle_clicking));
    replacement.watch(Watching::default()).unwrap();
    reads(0);
    drop(replacement);
    thread.quit_and_join(JOIN).unwrap();
}

#[test]
#[allow(unsafe_code)]
fn a_combination_another_program_holds_is_refused_by_name_and_the_rest_work() {
    let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let held = rare().toggle_recording;
    let vk = virtual_key(held.key()).unwrap();
    let modifiers = MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT;
    // SAFETY: a hotkey of this test's thread, removed below.
    assert_ne!(
        unsafe { RegisterHotKey(null_mut(), 0xB01, modifiers, u32::from(vk)) },
        0
    );
    let (thread, _) = started();
    let (sink, received) = sink();
    let reading = thread.remote().read_hotkeys(rare(), sink).unwrap();
    next(&received);
    let refused = vec![Unregistered {
        hotkey: held,
        why: NotRegistered::Taken,
    }];
    assert_eq!(
        next(&received),
        HotkeyEvent::Status(HotkeyStatus::Refused(refused))
    );
    assert!(!free(rare().toggle_clicking), "the others are registered");
    drop(reading);
    thread.quit_and_join(JOIN).unwrap();
    // SAFETY: registered above by this thread.
    unsafe { UnregisterHotKey(null_mut(), 0xB01) };
}

#[test]
fn the_windows_end_stops_the_reading_and_frees_the_hotkeys() {
    let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
    let (thread, _) = started();
    let (sink, received) = sink();
    let reading = thread.remote().read_hotkeys(rare(), sink).unwrap();
    next(&received);
    let stops = Watching {
        release: false,
        stops: vec![Action::ToggleClicking],
    };
    reading.watch(stops).unwrap();
    reads(1);
    thread.quit_and_join(JOIN).unwrap();
    assert_eq!(raw_registrations(), 0);
    assert!(free(rare().toggle_clicking));
    drop(reading);
}
