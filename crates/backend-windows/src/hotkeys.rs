//! Hotkeys on the message thread (M5 brief decision 9): `RegisterHotKey`
//! with `MOD_NOREPEAT` for the reader's bindings, and raw keyboard input,
//! registered only while core's [`Registered`] needs it (invariant N5).
//!
//! The runtime's reader asks for its hotkeys and says what the controller
//! wants the keyboard watched for; both are levels in a slot the thread
//! reads when told to, so a request that could not be posted is caught up
//! by the next. Each reader has an id, so the end of a reader that was
//! replaced never removes its replacement's hotkeys.
//!
//! What the hotkeys and the keyboard say goes straight to the reader's
//! sender, in the order the thread hears it, never through the message
//! thread's handler, which can wait seconds on a sleep or a logoff. Events
//! that carry the program's marker are dropped before anything sees them
//! (decision 8), and nothing here logs a key.

use std::cell::RefCell;
use std::io;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError};

use auto_clicker_core::hotkeys::{
    Action, Binding, Bindings, HotkeyEvent, HotkeyStatus, Keyboard, Modifier, NotRegistered,
    Registered, Triggers, Unregistered, Watching,
};
use auto_clicker_core::input::Key;
use auto_clicker_core::log;
use windows_sys::Win32::Foundation::{
    ERROR_HOTKEY_ALREADY_REGISTERED, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MAPVK_VSC_TO_VK_EX, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    MapVirtualKeyW, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    RIDEV_INPUTSINK, RIDEV_REMOVE, RIM_TYPEKEYBOARD, RegisterRawInputDevices,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, MSG, PM_REMOVE, PeekMessageW, RI_KEY_BREAK, RI_KEY_E0, WM_APP, WM_HOTKEY,
    WM_INPUT,
};

use crate::message_thread::Remote;
use crate::output::keys::{Code, code};
use crate::output::{MARKER, received_key};

/// Posted when the slot changed: the thread reads it.
const WM_HOTKEYS_CHANGED: u32 = WM_APP + 5;

/// The generic desktop page's keyboard usage, for raw input.
const KEYBOARD: (u16, u16) = (1, 6);

/// Gives each reader its id.
static READERS: AtomicU64 = AtomicU64::new(0);

/// What the runtime wants of the thread: the reader whose hotkeys are
/// registered, and what the keyboard is watched for.
#[derive(Default)]
pub(crate) struct Wanted {
    reader: Option<Wish>,
}

/// Where a reader's events go: to the controller, or a test.
pub type Sink = Arc<dyn Fn(HotkeyEvent) + Send + Sync>;

#[derive(Clone)]
struct Wish {
    id: u64,
    bindings: Bindings,
    send: Sink,
    watching: Watching,
}

/// One reader's hotkeys, registered on the message thread while this
/// lives.
pub struct HotkeyReading {
    remote: Remote,
    id: u64,
}

impl Remote {
    /// Registers `bindings` as hotkeys on the message thread, in place of
    /// any earlier reader's; what they do, and what the keyboard read for
    /// them says, goes to `send`. Their triggers and whether Windows took
    /// them come first.
    ///
    /// # Errors
    /// The window is gone, or Windows refused the post (its queue is full).
    pub fn read_hotkeys(&self, bindings: Bindings, send: Sink) -> io::Result<HotkeyReading> {
        let id = READERS.fetch_add(1, Ordering::Relaxed);
        self.wanted(|wanted| {
            wanted.reader = Some(Wish {
                id,
                bindings,
                send,
                watching: Watching::default(),
            });
        });
        let reading = HotkeyReading {
            remote: self.clone(),
            id,
        };
        self.post(WM_HOTKEYS_CHANGED, 0, 0)?;
        Ok(reading)
    }

    fn wanted(&self, change: impl FnOnce(&mut Wanted)) {
        change(
            &mut self
                .hotkeys()
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
    }
}

impl HotkeyReading {
    /// The controller now wants the keyboard read for `watching`.
    ///
    /// # Errors
    /// The window is gone, or Windows refused the post; the next post
    /// brings the thread up to date.
    pub fn watch(&self, watching: Watching) -> io::Result<()> {
        self.remote.wanted(|wanted| {
            if let Some(wish) = wanted.reader.as_mut().filter(|wish| wish.id == self.id) {
                wish.watching = watching;
            }
        });
        self.remote.post(WM_HOTKEYS_CHANGED, 0, 0)
    }
}

impl Drop for HotkeyReading {
    /// Unregisters the hotkeys, unless another reader took over already.
    fn drop(&mut self) {
        self.remote.wanted(|wanted| {
            if wanted
                .reader
                .as_ref()
                .is_some_and(|wish| wish.id == self.id)
            {
                wanted.reader = None;
            }
        });
        // A window that is gone took the hotkeys with it; a full queue is
        // caught up by the next post, or by the window's end.
        let _ = self.remote.post(WM_HOTKEYS_CHANGED, 0, 0);
    }
}

/// The hotkeys registered on this thread, for one reader.
struct Active {
    id: u64,
    send: Sink,
    /// The hotkey id Windows reports and the action, for each one it took.
    ids: Vec<(i32, Action)>,
    watching: Watching,
    registered: Registered,
}

thread_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
}

/// The slot changed: registers the wanted reader's hotkeys in place of the
/// old one's, and passes on what the keyboard is watched for.
fn changed(hwnd: HWND, remote: &Remote) {
    // Copied out, so registering and logging never hold up a runtime that
    // takes the slot.
    let wish = remote
        .hotkeys()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .reader
        .clone();
    let current = ACTIVE.with_borrow(|active| active.as_ref().map(|active| active.id));
    if current != wish.as_ref().map(|wish| wish.id) {
        end(hwnd);
        if let Some(wish) = &wish {
            ACTIVE.set(Some(register(hwnd, wish)));
        }
    }
    let watching = wish.map(|wish| wish.watching).unwrap_or_default();
    with_active(hwnd, |active, keyboard| {
        if active.watching == watching {
            return Vec::new();
        }
        active.watching = watching.clone();
        active.registered.watch(watching, keyboard)
    });
}

/// Unregisters every hotkey and stops reading the keyboard: the reader is
/// gone, or the window is.
pub(crate) fn end(hwnd: HWND) {
    let Some(mut active) = ACTIVE.take() else {
        return;
    };
    active.registered.end(&mut RawKeyboard(hwnd));
    for &(id, _) in &active.ids {
        // SAFETY: `hwnd` is this thread's window, which registered `id`.
        // A failure leaves a hotkey that goes with the window.
        #[allow(unsafe_code)]
        unsafe {
            UnregisterHotKey(hwnd, id)
        };
    }
}

/// What the window answers to a hotkey's or the keyboard's message;
/// `None` for any other message, and `Some(None)` to leave it to
/// `DefWindowProc` after.
pub(crate) fn answer(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<Option<LRESULT>> {
    match message {
        // A key press is never a wake to log (invariant N5).
        WM_HOTKEY => hotkey(hwnd, wparam),
        // Left to the default as well, which frees the event's data.
        WM_INPUT => {
            input(hwnd, lparam);
            return Some(None);
        }
        WM_HOTKEYS_CHANGED => Remote::on_thread(|remote| changed(hwnd, remote)),
        _ => return None,
    }
    Some(Some(0))
}

/// A `WM_HOTKEY` with this id. Raw input already queued goes first: it
/// happened before, and the key state the hotkey reads is newer than it.
fn hotkey(hwnd: HWND, id: WPARAM) {
    drain_input(hwnd);
    with_active(hwnd, |active, keyboard| {
        let action = active
            .ids
            .iter()
            .find(|&&(registered, _)| usize::try_from(registered).ok() == Some(id))
            .map(|&(_, action)| action);
        match action {
            Some(action) => active.registered.hotkey(action, keyboard),
            None => Vec::new(),
        }
    });
}

/// A `WM_INPUT`; the window procedure leaves it to `DefWindowProc` after.
fn input(hwnd: HWND, lparam: LPARAM) {
    let Some((code, down)) = key_event(lparam) else {
        return;
    };
    with_active(hwnd, |active, keyboard| {
        active.registered.key(code, down, keyboard)
    });
}

/// Handles every `WM_INPUT` waiting in the queue, in order.
#[allow(unsafe_code)]
fn drain_input(hwnd: HWND) {
    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is a live, writable `MSG`; only this window's
        // `WM_INPUT` is taken.
        let got = unsafe { PeekMessageW(&mut message, hwnd, WM_INPUT, WM_INPUT, PM_REMOVE) };
        if got == 0 {
            return;
        }
        input(hwnd, message.lParam);
        // SAFETY: the message as Windows queued it; its default handling
        // frees the raw input's data.
        unsafe { DefWindowProcW(hwnd, WM_INPUT, message.wParam, message.lParam) };
    }
}

/// Runs `step` on the active hotkeys, then sends what it says.
fn with_active(hwnd: HWND, step: impl FnOnce(&mut Active, &mut RawKeyboard) -> Vec<HotkeyEvent>) {
    let sent = ACTIVE.with_borrow_mut(|active| {
        let active = active.as_mut()?;
        let events = step(active, &mut RawKeyboard(hwnd));
        Some((Arc::clone(&active.send), events))
    });
    if let Some((send, events)) = sent {
        events.into_iter().for_each(|event| send(event));
    }
}

/// Registers each of `wish`'s bindings, and tells its reader the triggers
/// and which Windows refused.
fn register(hwnd: HWND, wish: &Wish) -> Active {
    let mut ids = Vec::new();
    let mut taken = Vec::new();
    let mut refused = Vec::new();
    for (index, (action, binding)) in wish.bindings.actions().into_iter().enumerate() {
        let id = i32::try_from(index).unwrap_or(i32::MAX).saturating_add(1);
        match register_one(hwnd, id, binding) {
            Ok(()) => {
                ids.push((id, action));
                taken.push((action, binding));
            }
            Err(why) => refused.push(Unregistered {
                hotkey: binding,
                why,
            }),
        }
    }
    // A refused binding's key stays refused too: the safe side, should
    // another program's hotkey on it be the user's.
    (wish.send)(HotkeyEvent::Triggers(Triggers::bound(&wish.bindings)));
    let status = if refused.is_empty() {
        HotkeyStatus::Available
    } else {
        HotkeyStatus::Refused(refused)
    };
    (wish.send)(HotkeyEvent::Status(status));
    Active {
        id: wish.id,
        send: Arc::clone(&wish.send),
        ids,
        watching: Watching::default(),
        registered: Registered::new(taken),
    }
}

#[allow(unsafe_code)]
fn register_one(hwnd: HWND, id: i32, binding: Binding) -> Result<(), NotRegistered> {
    let Some(vk) = virtual_key(binding.key()) else {
        return Err(NotRegistered::Failed(
            "its key has no virtual key on this keyboard layout"
                .to_owned()
                .into(),
        ));
    };
    let modifiers = binding
        .modifiers()
        .iter()
        .fold(MOD_NOREPEAT, |all, modifier| all | modifier_flag(modifier));
    // SAFETY: `hwnd` is this thread's live window; no pointers.
    if unsafe { RegisterHotKey(hwnd, id, modifiers, u32::from(vk)) } != 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    let taken = i64::from(ERROR_HOTKEY_ALREADY_REGISTERED);
    if err.raw_os_error().map(i64::from) == Some(taken) {
        Err(NotRegistered::Taken)
    } else {
        Err(NotRegistered::Failed(err.to_string().into()))
    }
}

fn modifier_flag(modifier: Modifier) -> u32 {
    match modifier {
        Modifier::Ctrl => MOD_CONTROL,
        Modifier::Shift => MOD_SHIFT,
        Modifier::Alt => MOD_ALT,
        Modifier::Meta => MOD_WIN,
    }
}

/// The virtual key Windows gives `key` on the layout in use: left and
/// right modifiers apart. `None` for a key without one.
#[allow(unsafe_code)]
fn virtual_key(key: Key) -> Option<u16> {
    match code(key)? {
        Code::Virtual(vk) => Some(vk),
        Code::Scan { scan, extended } => {
            let scan = u32::from(scan) | if extended { 0xE000 } else { 0 };
            // SAFETY: plain call.
            let vk = unsafe { MapVirtualKeyW(scan, MAPVK_VSC_TO_VK_EX) };
            u16::try_from(vk).ok().filter(|&vk| vk != 0)
        }
    }
}

/// The key a `WM_INPUT` reports, and whether it went down; `None` for
/// anything but an unmarked keyboard event of a key the program knows.
#[allow(unsafe_code)]
fn key_event(lparam: LPARAM) -> Option<(u16, bool)> {
    let mut input = RAWINPUT::default();
    let mut size = u32::try_from(size_of::<RAWINPUT>()).ok()?;
    let header = u32::try_from(size_of::<RAWINPUTHEADER>()).ok()?;
    // SAFETY: `lparam` is the handle the message carries; `input` is
    // `size` writable bytes, room for any keyboard or mouse event.
    let read = unsafe {
        GetRawInputData(
            lparam as HRAWINPUT,
            RID_INPUT,
            (&raw mut input).cast(),
            &mut size,
            header,
        )
    };
    let failed = read == u32::MAX || read == 0;
    if failed {
        return None;
    }
    if input.header.dwType != RIM_TYPEKEYBOARD {
        return None;
    }
    // SAFETY: a keyboard event, so the union holds its keyboard part.
    let keyboard = unsafe { input.data.keyboard };
    if keyboard.ExtraInformation == MARKER {
        return None;
    }
    let flags = u32::from(keyboard.Flags);
    let scan = u8::try_from(keyboard.MakeCode).ok()?;
    let key = received_key(scan, flags & RI_KEY_E0 != 0, keyboard.VKey)?;
    Some((key.code(), flags & RI_KEY_BREAK == 0))
}

/// Raw keyboard input for this thread's window.
struct RawKeyboard(HWND);

impl RawKeyboard {
    #[allow(unsafe_code)]
    fn register(flags: u32, target: HWND) -> io::Result<()> {
        let device = RAWINPUTDEVICE {
            usUsagePage: KEYBOARD.0,
            usUsage: KEYBOARD.1,
            dwFlags: flags,
            hwndTarget: target,
        };
        let size = u32::try_from(size_of::<RAWINPUTDEVICE>()).unwrap_or(u32::MAX);
        // SAFETY: one device, fully set, alive for the call.
        if unsafe { RegisterRawInputDevices(&device, 1, size) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Keyboard for RawKeyboard {
    /// Reads every keyboard, whatever window is in front.
    fn start(&mut self) -> io::Result<()> {
        RawKeyboard::register(RIDEV_INPUTSINK, self.0).inspect_err(|err| {
            log::line(format_args!(
                "auto-clicker: the keyboard could not be read for the hotkeys: {err}"
            ));
        })
    }

    fn stop(&mut self) {
        if let Err(err) = RawKeyboard::register(RIDEV_REMOVE, null_mut()) {
            log::line(format_args!(
                "auto-clicker: the keyboard's registration could not be removed: {err}"
            ));
        }
    }

    #[allow(unsafe_code)]
    fn held(&self, code: u16) -> bool {
        let Some(vk) = Key::new(code).and_then(virtual_key) else {
            return false;
        };
        // SAFETY: plain call. The high bit says the key is down.
        unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
    }
}

#[cfg(test)]
mod tests;
