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

use std::cell::{Cell, RefCell};
use std::io;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError};

use auto_clicker_core::hotkeys::{
    Action, Binding, Bindings, HotkeyEvent, HotkeyStatus, Keyboard, Modifier, NotRegistered,
    Registered, Triggers, Unregistered, Watching,
};
use auto_clicker_core::input::{Held, Key};
use auto_clicker_core::log;
use windows_sys::Win32::Foundation::{
    ERROR_HOTKEY_ALREADY_REGISTERED, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardLayoutList, HKL, MAPVK_VK_TO_VSC_EX, MAPVK_VSC_TO_VK_EX, MOD_ALT,
    MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, MapVirtualKeyExW, MapVirtualKeyW,
    RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    RIDEV_DEVNOTIFY, RIDEV_INPUTSINK, RIDEV_REMOVE, RIM_TYPEKEYBOARD, RegisterRawInputDevices,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, EVENT_SYSTEM_DESKTOPSWITCH, GIDC_REMOVAL, MSG, PM_REMOVE, PeekMessageW,
    PostQuitMessage, RI_KEY_BREAK, RI_KEY_E0, WINEVENT_OUTOFCONTEXT, WM_APP, WM_HOTKEY, WM_INPUT,
    WM_INPUT_DEVICE_CHANGE, WM_QUIT,
};

use crate::message_thread::Remote;
use crate::output::keys::{self, Code, code};
use crate::output::{MARKER, received_key};
use crate::watchdog::Table;

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
    own: Arc<Table>,
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
    /// them come first. `own` is the table of what the program holds down,
    /// whose keys the key state is never read for.
    ///
    /// # Errors
    /// The window is gone, or Windows refused the post (its queue is full).
    pub fn read_hotkeys(
        &self,
        bindings: Bindings,
        own: Arc<Table>,
        send: Sink,
    ) -> io::Result<HotkeyReading> {
        let id = READERS.fetch_add(1, Ordering::Relaxed);
        self.wanted(|wanted| {
            wanted.reader = Some(Wish {
                id,
                bindings,
                own,
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
    own: Arc<Table>,
    send: Sink,
    /// The hotkey id Windows reports and the action, for each one it took.
    ids: Vec<(i32, Action)>,
    watching: Watching,
    registered: Registered,
}

thread_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
    /// The hook on desktop switches while hotkeys are registered, and the
    /// window it reports to: a handle and a window of this thread.
    static DESKTOP_HOOK: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

/// Hears the input desktop switch (to UAC's secure desktop and back, say),
/// which sends no session notice: what the keyboard did meanwhile went
/// there, so the keys followed are forgotten as on a lock. Out of context,
/// so it arrives through this thread's message loop and costs nothing
/// while no switch happens.
#[allow(unsafe_code)]
fn watch_desktop(hwnd: HWND) {
    if DESKTOP_HOOK.get().is_some() {
        return;
    }
    // SAFETY: an out-of-context hook of this thread's, its procedure a
    // plain function; removed in `unwatch_desktop`.
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_DESKTOPSWITCH,
            EVENT_SYSTEM_DESKTOPSWITCH,
            null_mut(),
            Some(desktop_switched),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.is_null() {
        log::line(format_args!(
            "auto-clicker: desktop switches cannot be heard: {}",
            io::Error::last_os_error()
        ));
        return;
    }
    DESKTOP_HOOK.set(Some((hook as usize, hwnd as usize)));
}

#[allow(unsafe_code)]
fn unwatch_desktop() {
    if let Some((hook, _)) = DESKTOP_HOOK.take() {
        // SAFETY: the hook `watch_desktop` set on this thread.
        unsafe { UnhookWinEvent(hook as HWINEVENTHOOK) };
    }
}

/// The hook's procedure: Windows calls it on this thread, in its message
/// loop.
#[allow(unsafe_code)]
unsafe extern "system" fn desktop_switched(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _window: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if let Some((_, hwnd)) = DESKTOP_HOOK.get() {
        session_changed(hwnd as HWND);
    }
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
            watch_desktop(hwnd);
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
    unwatch_desktop();
    let Some(mut active) = ACTIVE.take() else {
        return;
    };
    active
        .registered
        .end(&mut RawKeyboard::new(hwnd, &active.own));
    for &(id, _) in &active.ids {
        // SAFETY: `hwnd` is this thread's window, which registered `id`.
        // A failure leaves a hotkey that goes with the window.
        #[allow(unsafe_code)]
        unsafe {
            UnregisterHotKey(hwnd, id)
        };
    }
}

/// The session was locked, unlocked or switched, the input desktop
/// switched, the system slept or woke, or a keyboard went away: the keys
/// followed are forgotten (core's [`Registered::session_changed`]).
pub(crate) fn session_changed(hwnd: HWND) {
    with_active(hwnd, |active, keyboard| {
        active.registered.session_changed(keyboard)
    });
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
        // A keyboard went away: what it held never comes up.
        WM_INPUT_DEVICE_CHANGE if u32::try_from(wparam) == Ok(GIDC_REMOVAL) => {
            session_changed(hwnd);
        }
        _ => return None,
    }
    Some(Some(0))
}

/// A `WM_HOTKEY` with this id. Raw input already queued goes first: Windows
/// hands a posted hotkey out ahead of input that came before it (the
/// modifiers' downs, an earlier press's releases), and the key state the
/// hotkey reads is newer than all of it. A hotkey that fires never shows
/// its key's press in raw input (measured in the WN01 VM, build
/// 26300.9974), so this never counts the hotkey's own press first.
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
    let Some(event) = key_event(lparam) else {
        return;
    };
    with_active(hwnd, |active, keyboard| match event {
        KeyEvent::User { code, down } => active.registered.key(code, down, keyboard),
        KeyEvent::OwnRelease(code) => active.registered.own_release(code, keyboard),
        KeyEvent::OwnPress => Vec::new(),
    });
}

/// A keyboard event raw input reported.
enum KeyEvent {
    /// The user's key went down or came up.
    User { code: u16, down: bool },
    /// The program let go of a key it held: whatever the key state said
    /// of it, it is up now.
    OwnRelease(u16),
    /// The program pressed a key: never seen (decision 8).
    OwnPress,
}

/// Handles every `WM_INPUT` waiting in the queue, in order. Each takes
/// microseconds, so no keyboard outruns the loop; only a program flooding
/// the input stream faster could, which holds up the whole desktop as
/// well. A `WM_QUIT`, which `PeekMessage` takes whatever the filter, is
/// posted again for the thread's loop, which ends on it.
#[allow(unsafe_code)]
fn drain_input(hwnd: HWND) {
    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is a live, writable `MSG`; only this window's
        // `WM_INPUT` is taken, and `WM_QUIT`.
        let got = unsafe { PeekMessageW(&mut message, hwnd, WM_INPUT, WM_INPUT, PM_REMOVE) };
        if got == 0 {
            return;
        }
        if message.message == WM_QUIT {
            // The exit code `PostQuitMessage` put in `wParam`.
            let code = i32::try_from(message.wParam).unwrap_or(i32::MAX);
            // SAFETY: plain call; the thread's loop ends on it.
            unsafe { PostQuitMessage(code) };
            return;
        }
        input(hwnd, message.lParam);
        // SAFETY: the message as Windows queued it; its default handling
        // frees the raw input's data.
        unsafe { DefWindowProcW(hwnd, WM_INPUT, message.wParam, message.lParam) };
    }
}

/// Runs `step` on the active hotkeys, then sends what it says.
fn with_active(
    hwnd: HWND,
    step: impl FnOnce(&mut Active, &mut RawKeyboard<'_>) -> Vec<HotkeyEvent>,
) {
    let sent = ACTIVE.with_borrow_mut(|active| {
        let active = active.as_mut()?;
        let own = Arc::clone(&active.own);
        let mut keyboard = RawKeyboard::new(hwnd, &own);
        let events = step(active, &mut keyboard);
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
        let id = hotkey_id(wish.id, index);
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
    // A refused binding's keys stay refused too: the safe side, should
    // another program's hotkey on it be the user's.
    let triggers = Triggers::bound(&wish.bindings, |binding| setting_off(binding.key()));
    (wish.send)(HotkeyEvent::Triggers(triggers));
    let status = if refused.is_empty() {
        HotkeyStatus::Available
    } else {
        HotkeyStatus::Refused(refused)
    };
    (wish.send)(HotkeyEvent::Status(status));
    Active {
        id: wish.id,
        own: Arc::clone(&wish.own),
        send: Arc::clone(&wish.send),
        ids,
        watching: Watching::default(),
        registered: Registered::new(taken, |binding| set_off_here(binding.key())),
    }
}

/// The hotkey ids a program may use run from 0 to 0xBFFF.
const HOTKEY_IDS: u64 = 0xC000;

/// The id of a reader's `index`th hotkey: each reader its own three, so a
/// `WM_HOTKEY` still queued from a reader that was replaced matches none of
/// its replacement's (they come round again only after 16,383 readers).
fn hotkey_id(reader: u64, index: usize) -> i32 {
    let block = reader % (HOTKEY_IDS / 3 - 1);
    let index = u64::try_from(index).unwrap_or(0) % 3;
    // Below 0xC000, so it fits.
    i32::try_from(block * 3 + index + 1).unwrap_or(1)
}

/// Keys Windows reports in raw input otherwise than they are sent, which
/// WN09 measures before reading them back: SysRq (Alt+PrintScreen comes as
/// plain 0x54), Pause (Ctrl+Pause comes as Break), Hangeul and Hanja. A
/// hotkey on one could not be stopped behind a game that turns hotkeys
/// off, so it is not registered yet.
const READ_BACK_LATER: [u16; 4] = [99, 119, 122, 123];

/// Why `binding` is not registered yet, if it is not.
fn not_yet(binding: Binding) -> Option<NotRegistered> {
    READ_BACK_LATER.contains(&binding.key().code()).then(|| {
        NotRegistered::Failed(
            "its key cannot be read back on Windows yet"
                .to_owned()
                .into(),
        )
    })
}

#[allow(unsafe_code)]
fn register_one(hwnd: HWND, id: i32, binding: Binding) -> Result<(), NotRegistered> {
    if let Some(why) = not_yet(binding) {
        return Err(why);
    }
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
    virtual_key_in(key, null_mut())
}

/// The virtual key `layout` gives `key`; the thread's own layout when it is
/// null.
#[allow(unsafe_code)]
fn virtual_key_in(key: Key, layout: HKL) -> Option<u16> {
    match code(key)? {
        Code::Virtual(vk) => Some(vk),
        Code::Scan { scan, extended } => {
            let scan = u32::from(scan) | if extended { 0xE000 } else { 0 };
            // SAFETY: plain call; a null layout is the thread's own.
            let vk = unsafe { MapVirtualKeyExW(scan, MAPVK_VSC_TO_VK_EX, layout) };
            u16::try_from(vk).ok().filter(|&vk| vk != 0)
        }
    }
}

/// The keyboard layouts the user has installed, which the window in front
/// may use rather than this thread's.
#[allow(unsafe_code)]
fn layouts() -> Vec<HKL> {
    // SAFETY: a null buffer asks for the count alone.
    let count = unsafe { GetKeyboardLayoutList(0, null_mut()) };
    let mut layouts = vec![null_mut(); usize::try_from(count).unwrap_or(0)];
    // SAFETY: room for `count` handles; Windows writes at most that many.
    let written = unsafe { GetKeyboardLayoutList(count, layouts.as_mut_ptr()) };
    layouts.truncate(usize::try_from(written).unwrap_or(0));
    layouts
}

/// The keys whose press sets off a hotkey on `key`: every key the program
/// can send that one of the installed layouts gives the virtual key it was
/// registered with (Enter and keypad Enter; Home and keypad 7, which
/// NumLock decides between; Z for a Y on a German layout), and `key`
/// itself whatever it maps to. A layout installed later is not covered
/// until the hotkeys are registered again.
/// The keys that set off a hotkey on `key` on this thread's layout, the one
/// it was registered with: what raw input matches a stop against, so a key
/// another layout maps there (Z for a Y) stops nothing Windows would not.
fn set_off_here(key: Key) -> Vec<Key> {
    let Some(vk) = virtual_key(key) else {
        return vec![key];
    };
    let mut keys = vec![key];
    keys.extend(keys::sendable().filter(|&other| other != key && virtual_key(other) == Some(vk)));
    keys
}

fn setting_off(key: Key) -> Vec<Key> {
    let Some(vk) = virtual_key(key) else {
        return vec![key];
    };
    let layouts = layouts();
    let mut keys = vec![key];
    keys.extend(keys::sendable().filter(|&other| {
        other != key
            && (virtual_key(other) == Some(vk)
                || layouts
                    .iter()
                    .any(|&layout| virtual_key_in(other, layout) == Some(vk)))
    }));
    keys
}

/// The key event a `WM_INPUT` reports; `None` for anything but a keyboard
/// event of a key the program knows. What carries the program's marker is
/// told apart, and only its releases say anything.
#[allow(unsafe_code)]
fn key_event(lparam: LPARAM) -> Option<KeyEvent> {
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
    let flags = u32::from(keyboard.Flags);
    let down = flags & RI_KEY_BREAK == 0;
    if keyboard.ExtraInformation == MARKER && down {
        return Some(KeyEvent::OwnPress);
    }
    let (scan, extended) = scan_code(keyboard.MakeCode, flags, keyboard.VKey)?;
    let code = received_key(scan, extended, keyboard.VKey)?.code();
    if keyboard.ExtraInformation == MARKER {
        return Some(KeyEvent::OwnRelease(code));
    }
    Some(KeyEvent::User { code, down })
}

/// The set-1 scan code of a raw keyboard event, and whether it is an `E0`
/// key. Another program may send a key by its virtual key alone, with no
/// scan code: the layout's scan code for that virtual key stands in.
#[allow(unsafe_code)]
fn scan_code(made: u16, flags: u32, vk: u16) -> Option<(u8, bool)> {
    if made != 0 {
        return Some((u8::try_from(made).ok()?, flags & RI_KEY_E0 != 0));
    }
    // SAFETY: plain call.
    let mapped = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC_EX) };
    let extended = mapped & 0xff00 == 0xe000;
    Some((u8::try_from(mapped & 0xff).ok()?, extended)).filter(|&(scan, _)| scan != 0)
}

/// Raw keyboard input for this thread's window, and the user's key state:
/// a key the program holds (in the watchdog's table) is never read as the
/// user's. Its marked release still reaches core, but only to take back
/// what the key state said of it.
struct RawKeyboard<'a> {
    window: HWND,
    own: &'a Table,
}

impl<'a> RawKeyboard<'a> {
    fn new(window: HWND, own: &'a Table) -> RawKeyboard<'a> {
        RawKeyboard { window, own }
    }

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

impl Keyboard for RawKeyboard<'_> {
    /// Reads every keyboard, whatever window is in front.
    fn start(&mut self) -> io::Result<()> {
        RawKeyboard::register(RIDEV_INPUTSINK | RIDEV_DEVNOTIFY, self.window).inspect_err(|err| {
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
        let Some(key) = Key::new(code) else {
            return false;
        };
        let Some(vk) = virtual_key(key) else {
            return false;
        };
        // SAFETY: plain call. The high bit says the key is down.
        let down = unsafe { GetAsyncKeyState(i32::from(vk)) < 0 };
        // Asked after the state: the program marks a key before it presses
        // it. A release it sent that the state does not show yet is caught
        // by its marked event coming up.
        down && !self.own.owes(Held::Key(key))
    }
}

#[cfg(test)]
mod tests;
