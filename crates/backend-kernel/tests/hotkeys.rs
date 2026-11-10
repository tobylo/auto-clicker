//! Hand-run test of hotkeys against real kernel devices.
//!
//! Needs write access to `/dev/uinput` and read access to `/dev/input/event*`
//! (the `input` group). It creates throwaway virtual keyboards and presses
//! Ctrl+F22 to Ctrl+F24 on them, which no desktop binds; any session works,
//! and no window is needed. Leave Ctrl alone while it runs. From the
//! repository root:
//!
//! ```text
//! cargo test --locked -p auto-clicker-backend-kernel --test hotkeys -- --ignored --test-threads=1
//! ```

// clippy.toml exempts `#[test]` functions only; this whole file is test code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use auto_clicker_backend_kernel::{KernelBackend, watch_hotkeys};
use auto_clicker_core::hotkeys::{Action, Binding, Bindings, HotkeyEvent};
use auto_clicker_core::input::Key;
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, timeout, timeout_at};

/// Generous: a new device takes a moment to appear and be opened.
const DEVICE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to watch for a hotkey that must not fire.
const QUIET: Duration = Duration::from_secs(1);

fn bindings() -> Bindings {
    Bindings {
        toggle_clicking: Binding::parse("Ctrl+F24").unwrap(),
        toggle_recording: Binding::parse("Ctrl+F23").unwrap(),
        toggle_playback: Binding::parse("Ctrl+F22").unwrap(),
    }
}

const LEFTCTRL: u16 = 29;
const F24: u16 = 194;

/// A virtual keyboard that is not this program's: it stands for a real one.
struct Keyboard(VirtualDevice);

impl Keyboard {
    fn new(name: &str, keys: &[u16]) -> Keyboard {
        let mut set = AttributeSet::<KeyCode>::new();
        for &code in keys {
            set.insert(KeyCode(code));
        }
        let device = VirtualDevice::builder()
            .unwrap()
            .name(name)
            .with_keys(&set)
            .unwrap()
            .build()
            .unwrap();
        Keyboard(device)
    }

    fn full(name: &str) -> Keyboard {
        Keyboard::new(name, &[LEFTCTRL, F24])
    }

    fn key(&mut self, code: u16, value: i32) {
        let event = InputEvent::new(EventType::KEY.0, code, value);
        self.0.emit(&[event]).unwrap();
    }

    fn ctrl_f24(&mut self) {
        self.key(LEFTCTRL, 1);
        self.key(F24, 1);
        self.key(F24, 0);
        self.key(LEFTCTRL, 0);
    }
}

/// The hotkey reader, running beside the test on the same thread.
struct Reader {
    received: mpsc::UnboundedReceiver<HotkeyEvent>,
}

impl Reader {
    /// Waits up to `wait` for the next event that is not a status.
    async fn next_hotkey(&mut self, wait: Duration) -> Option<HotkeyEvent> {
        let deadline = Instant::now() + wait;
        loop {
            match timeout_at(deadline, self.received.recv()).await.ok()?? {
                HotkeyEvent::Status(_) => {}
                event => return Some(event),
            }
        }
    }

    /// Presses Ctrl+F24 on `keyboard` until it fires, which also shows the
    /// reader has opened the keyboard: a new device takes a moment.
    async fn fires_from(&mut self, keyboard: &mut Keyboard) {
        let deadline = Instant::now() + DEVICE_TIMEOUT;
        loop {
            assert!(Instant::now() < deadline, "Ctrl+F24 never fired");
            keyboard.ctrl_f24();
            if let Some(event) = self.next_hotkey(Duration::from_millis(200)).await {
                assert_eq!(event, HotkeyEvent::Fired(Action::ToggleClicking));
                let released = self.next_hotkey(QUIET).await;
                assert_eq!(released, Some(HotkeyEvent::ModifiersReleased));
                return;
            }
        }
    }

    /// Checks that nothing fires within [`QUIET`].
    async fn stays_quiet(&mut self) {
        let event = self.next_hotkey(QUIET).await;
        assert_eq!(event, None, "a hotkey fired");
    }
}

/// Runs `test` beside the reader, which knows this program's devices from
/// `backend`, then shuts the reader down and checks it ends at once.
async fn with_reader<F, Fut>(test: F)
where
    F: FnOnce(Reader, auto_clicker_core::input::Input<KernelBackend>) -> Fut,
    Fut: Future<Output = ()>,
{
    let input = KernelBackend::open().unwrap();
    let own = input.backend().devices().to_vec();
    let (events, received) = mpsc::unbounded_channel();
    let (stop, stopped) = oneshot::channel::<()>();
    let reader = watch_hotkeys(
        bindings(),
        &own,
        move |event| {
            let _ = events.send(event);
        },
        async move {
            let _ = stopped.await;
        },
    );
    let driven = async move {
        test(Reader { received }, input).await;
        stop.send(()).unwrap();
    };
    let both = async { tokio::join!(reader, driven) };
    timeout(Duration::from_secs(60), both)
        .await
        .expect("the reader did not end after shutdown");
}

#[tokio::test]
#[ignore = "needs /dev/uinput and the input group; run by hand"]
async fn another_keyboard_fires_and_ours_never_does() {
    with_reader(|mut reader, mut input| async move {
        let mut keyboard = Keyboard::full("auto-clicker test keyboard");
        reader.fires_from(&mut keyboard).await;
        // The same keys from this program's own keyboard.
        let ctrl = Key::from_name("leftctrl").unwrap();
        let f24 = Key::from_name("f24").unwrap();
        input.press_key(ctrl).unwrap();
        input.press_key(f24).unwrap();
        input.release_key(f24).unwrap();
        input.release_key(ctrl).unwrap();
        reader.stays_quiet().await;
    })
    .await;
}

#[tokio::test]
#[ignore = "needs /dev/uinput and the input group; run by hand"]
async fn ctrl_on_one_keyboard_and_f24_on_another_fires() {
    with_reader(|mut reader, _input| async move {
        // The keys first: the reader then polls them before the modifiers,
        // and only the events' timestamps keep Ctrl ahead of F24.
        let mut keys = Keyboard::new("auto-clicker test keys", &[F24]);
        let mut modifiers = Keyboard::new("auto-clicker test modifiers", &[LEFTCTRL]);
        let deadline = Instant::now() + DEVICE_TIMEOUT;
        loop {
            assert!(Instant::now() < deadline, "Ctrl+F24 never fired");
            modifiers.key(LEFTCTRL, 1);
            keys.key(F24, 1);
            keys.key(F24, 0);
            modifiers.key(LEFTCTRL, 0);
            if let Some(event) = reader.next_hotkey(Duration::from_millis(200)).await {
                assert_eq!(event, HotkeyEvent::Fired(Action::ToggleClicking));
                break;
            }
        }
    })
    .await;
}

#[tokio::test]
#[ignore = "needs /dev/uinput and the input group; run by hand"]
async fn a_keyboard_that_comes_back_fires_again() {
    with_reader(|mut reader, _input| async move {
        let mut keyboard = Keyboard::full("auto-clicker test keyboard");
        reader.fires_from(&mut keyboard).await;
        // Unplugged while Ctrl+F24 holds Ctrl: whether Ctrl came up is not
        // seen, so the release is lost rather than claimed.
        keyboard.key(LEFTCTRL, 1);
        keyboard.key(F24, 1);
        let fired = reader.next_hotkey(DEVICE_TIMEOUT).await;
        assert_eq!(fired, Some(HotkeyEvent::Fired(Action::ToggleClicking)));
        drop(keyboard);
        let lost = reader.next_hotkey(DEVICE_TIMEOUT).await;
        assert_eq!(lost, Some(HotkeyEvent::ReleaseLost));
        let mut again = Keyboard::full("auto-clicker test keyboard");
        reader.fires_from(&mut again).await;
    })
    .await;
}

#[tokio::test]
#[ignore = "needs /dev/uinput and the input group; run by hand"]
async fn unplugging_one_keyboard_leaves_the_other_working() {
    with_reader(|mut reader, _input| async move {
        let mut first = Keyboard::full("auto-clicker test keyboard 1");
        let mut second = Keyboard::full("auto-clicker test keyboard 2");
        reader.fires_from(&mut first).await;
        reader.fires_from(&mut second).await;
        drop(first);
        second.ctrl_f24();
        let fired = reader.next_hotkey(DEVICE_TIMEOUT).await;
        assert_eq!(fired, Some(HotkeyEvent::Fired(Action::ToggleClicking)));
    })
    .await;
}

#[tokio::test]
#[ignore = "needs /dev/uinput and the input group; run by hand"]
async fn keys_held_across_lost_events_never_fire() {
    const F21: u16 = 191;
    with_reader(|mut reader, _input| async move {
        let mut keyboard = Keyboard::new("auto-clicker test keyboard", &[LEFTCTRL, F24, F21]);
        reader.fires_from(&mut keyboard).await;
        // F24 before Ctrl, then far more events than the kernel queues
        // while the reader, on this same thread, cannot run: they are lost.
        keyboard.key(F24, 1);
        for _ in 0..2000 {
            keyboard.key(F21, 1);
            keyboard.key(F21, 0);
        }
        keyboard.key(LEFTCTRL, 1);
        reader.stays_quiet().await;
        keyboard.key(F24, 0);
        keyboard.key(LEFTCTRL, 0);
        reader.fires_from(&mut keyboard).await;
    })
    .await;
}
