//! The Win32 message thread (M5 brief decision 4, map D11): one thread that
//! owns the daemon's hidden window and sleeps in `GetMessage` until Windows
//! has something to say. It hands what it hears to one handler as
//! [`WindowEvent`]s and decides nothing itself.
//!
//! The window is top-level but never shown, not a message-only window:
//! Windows sends sleep and resume, logoff and the taskbar's restart only to
//! top-level windows. So it also hears what Windows tells every window (a
//! setting or the time changed); those come to the handler as
//! [`WindowEvent::Other`], so every wake can be told apart.
//!
//! The runtime reaches the thread by posting to the window rather than to
//! the thread, since a modal loop (a message box, a menu) drops messages
//! posted to the thread. The window's handle is shared under a lock and
//! cleared under it when the window is destroyed, so nothing posts to a
//! handle Windows may have given to another window since.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{
    HPOWERNOTIFY, POWERBROADCAST_SETTING, RegisterPowerSettingNotification,
    UnregisterPowerSettingNotification,
};
use windows_sys::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows_sys::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, KillTimer, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
    PBT_POWERSETTINGCHANGE, PostMessageW, PostQuitMessage, RegisterClassW, SetTimer,
    UnregisterClassW, WM_APP, WM_CLOSE, WM_DESTROY, WM_ENDSESSION, WM_POWERBROADCAST,
    WM_QUERYENDSESSION, WM_TIMER, WM_WTSSESSION_CHANGE, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
    WTS_CONSOLE_CONNECT, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_CONNECT, WTS_REMOTE_DISCONNECT,
    WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

use crate::message::wide;
use crate::session::{SessionState, query};

/// Posted by [`Remote::quit`]: destroy the window and end the thread.
const WM_QUIT_REQUEST: u32 = WM_APP + 1;
/// The timer that tries the session notices' registration again.
const RETRY_TIMER: usize = 1;
/// The waits between tries at registering for session notices: soon at
/// first, since early in a logon the service that sends them may not be
/// ready yet, then less often.
const RETRY_DELAYS: [Duration; 6] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(30),
];
/// The display state in a `GUID_CONSOLE_DISPLAY_STATE` notice: off, on,
/// dimmed. Dimmed counts as on.
const DISPLAY_OFF: u32 = 0;

/// What Windows told the message thread.
#[derive(Debug)]
pub enum WindowEvent {
    /// The session's state, read once its notices are registered for: at
    /// start, and after a registration that failed succeeded later.
    SessionRead(io::Result<SessionState>),
    /// Registering for session notices failed; it is tried again later, and
    /// no session notice arrives until it succeeds.
    SessionUnwatched(io::Error),
    /// A session notice.
    Session(SessionChange),
    /// The computer is about to sleep. Windows suspends once the handler
    /// returns, or about two seconds after it was told.
    Suspend,
    /// The computer woke from sleep (or a sleep was called off).
    Resume,
    /// The display turned off (`false`) or on (`true`). Windows sends the
    /// current state once the notice is registered for.
    Display(bool),
    /// Registering for the display's state failed; it is not followed.
    DisplayUnwatched(io::Error),
    /// The user's session is ending: logoff, shutdown, or the Restart
    /// Manager closing the program. Windows may end the process at any time
    /// once the handler returns.
    EndSession,
    /// Something asked the window to close (`taskkill` without `/F`).
    Close,
    /// A message the thread does not act on, by its number: what woke it.
    Other(u32),
    /// The thread left its message loop and is about to end; always the
    /// last event. `failure` says why, when it was not told to quit.
    Ended { failure: Option<String> },
}

/// A session notice, as `WM_WTSSESSION_CHANGE` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionChange {
    Lock,
    Unlock,
    /// The session was connected to the console or a remote desktop: a
    /// switch back to this user.
    Connect,
    /// The session was disconnected from the console or a remote desktop:
    /// a switch to another user.
    Disconnect,
}

impl SessionChange {
    /// The change a `WM_WTSSESSION_CHANGE` with this `wParam` reports; none
    /// for the ones the program does not follow (logon, logoff, remote
    /// control).
    fn from_code(code: usize) -> Option<SessionChange> {
        let code = u32::try_from(code).ok()?;
        match code {
            WTS_SESSION_LOCK => Some(SessionChange::Lock),
            WTS_SESSION_UNLOCK => Some(SessionChange::Unlock),
            WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => Some(SessionChange::Connect),
            WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT => Some(SessionChange::Disconnect),
            _ => None,
        }
    }
}

/// The handler every event goes to, on the message thread. It may block
/// briefly, with a deadline of its own: the thread handles nothing else
/// meanwhile.
type Handler = Box<dyn FnMut(WindowEvent) + Send>;

/// Why joining the thread failed.
#[derive(Debug)]
pub enum JoinError {
    /// It had not ended by the deadline; it is left to end with the process.
    TimedOut,
    /// It panicked outside its handler and window procedure.
    Panicked,
}

/// The running message thread: post to it, and end it.
pub struct MessageThread {
    remote: Remote,
    /// Signalled (or dropped) on the thread's way out.
    done: mpsc::Receiver<()>,
    thread: JoinHandle<()>,
}

/// Where the runtime reaches the message thread. Cheap to clone and to
/// send to another thread.
#[derive(Clone)]
pub struct Remote(Arc<Mutex<Option<Window>>>);

/// A window handle that may cross threads: Windows lets any thread post to
/// any window.
#[derive(Clone, Copy)]
struct Window(HWND);

// SAFETY: a window handle is an identifier, not a pointer this program
// dereferences; posting to it from any thread is what Windows documents.
// Only the message thread, which owns the window, destroys it, and it
// clears the shared copy first, under the same lock posters take.
#[allow(unsafe_code)]
unsafe impl Send for Window {}

impl MessageThread {
    /// Starts the thread: its window, the session and display notices, then
    /// its message loop, sending every event to `handler`. Returns once the
    /// window exists, or when `ready_within` passes.
    ///
    /// # Errors
    /// The thread or its window could not be created, or was not ready in
    /// time (then it is left to end with the process).
    pub fn spawn(
        handler: impl FnMut(WindowEvent) + Send + 'static,
        ready_within: Duration,
    ) -> io::Result<MessageThread> {
        let remote = Remote(Arc::new(Mutex::new(None)));
        let (ready, readiness) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let shared = remote.clone();
        let thread = thread::Builder::new()
            .name("message thread".to_owned())
            .spawn(move || run(Box::new(handler), &shared, &ready, finished))?;
        match readiness.recv_timeout(ready_within) {
            Ok(Ok(())) => Ok(MessageThread {
                remote,
                done,
                thread,
            }),
            Ok(Err(err)) => {
                // The thread returns right after reporting the failure.
                let _ = thread.join();
                Err(err)
            }
            Err(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the message thread's window was not ready in time",
            )),
        }
    }

    /// Where the runtime reaches the thread.
    pub fn remote(&self) -> Remote {
        self.remote.clone()
    }

    /// Tells the thread to quit and waits for it, at most `within`. Its
    /// window is destroyed on its way out, and with it everything the
    /// window registered for.
    ///
    /// # Errors
    /// [`JoinError::TimedOut`] when it is still running at the deadline,
    /// [`JoinError::Panicked`] when it panicked.
    pub fn quit_and_join(self, within: Duration) -> Result<(), JoinError> {
        // A thread that already ended has no window left to post to.
        let _ = self.remote.quit();
        match self.done.recv_timeout(within) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.thread.join().map_err(|_| JoinError::Panicked)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Err(JoinError::TimedOut),
        }
    }
}

impl Remote {
    /// Asks the thread to destroy its window and end.
    ///
    /// # Errors
    /// The window is already gone, or Windows refused the post (its queue
    /// is full).
    pub fn quit(&self) -> io::Result<()> {
        self.post(WM_QUIT_REQUEST, 0, 0)
    }

    /// Posts `message` to the window, under the lock that its destruction
    /// takes.
    #[allow(unsafe_code)]
    fn post(&self, message: u32, wparam: WPARAM, lparam: LPARAM) -> io::Result<()> {
        let window = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(Window(hwnd)) = *window else {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "the message thread's window is gone",
            ));
        };
        // SAFETY: `hwnd` is the message thread's window, alive while the
        // lock is held (its destruction clears it under this lock first);
        // the message carries no pointer.
        let posted = unsafe { PostMessageW(hwnd, message, wparam, lparam) };
        if posted == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

thread_local! {
    /// The handler, while the thread runs. Taken while it is called, so a
    /// message that arrives meanwhile waits in `PENDING`.
    static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
    /// Events that arrived while the handler was busy.
    static PENDING: RefCell<VecDeque<WindowEvent>> = const { RefCell::new(VecDeque::new()) };
    /// The window's shared handle, cleared when it is destroyed.
    static SHARED: RefCell<Option<Remote>> = const { RefCell::new(None) };
    /// Whether the window is set up: messages before then (its creation's)
    /// and after its destruction are not reported as `Other`.
    static READY: Cell<bool> = const { Cell::new(false) };
    /// Whether session notices are registered for.
    static SESSION_WATCHED: Cell<bool> = const { Cell::new(false) };
    /// How many registration retries have been scheduled.
    static RETRIES: Cell<usize> = const { Cell::new(0) };
    /// The display notice's registration; 0 when there is none.
    static DISPLAY: Cell<HPOWERNOTIFY> = const { Cell::new(0) };
    /// Why the loop ended, when it was not told to quit.
    static FAILURE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Names each thread's window class apart, so tests can run several.
static CLASSES: AtomicU64 = AtomicU64::new(0);

/// The thread's body: the window, the loop, the cleanup, and the last
/// event, whatever happens in between.
fn run(
    handler: Handler,
    shared: &Remote,
    ready: &mpsc::Sender<io::Result<()>>,
    finished: mpsc::Sender<()>,
) {
    // Dropped last, so the joiner hears of the end only once all is done.
    let _finished = Finished(finished);
    HANDLER.set(Some(handler));
    SHARED.set(Some(shared.clone()));
    let class = class_name();
    let failure = match catch_unwind(AssertUnwindSafe(|| serve(&class, shared, ready))) {
        Ok(failure) => failure,
        Err(panic) => Some(Some(panicked(&*panic))),
    };
    // An `Ended` after a refused start would reach a controller that never
    // runs; only a thread that was ready reports it.
    if let Some(failure) = failure {
        emit(WindowEvent::Ended { failure });
    }
    HANDLER.set(None);
    SHARED.set(None);
}

/// Creates the window, reports readiness, and runs the loop; the loop's
/// failure, or `None` when the window could not be created (reported to
/// `ready` instead).
fn serve(
    class: &[u16],
    shared: &Remote,
    ready: &mpsc::Sender<io::Result<()>>,
) -> Option<Option<String>> {
    let hwnd = match create_window(class) {
        Ok(hwnd) => hwnd,
        Err(err) => {
            let _ = ready.send(Err(err));
            return None;
        }
    };
    *shared.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(Window(hwnd));
    watch_session(hwnd);
    watch_display(hwnd);
    READY.set(true);
    // The spawner may have given up already; the thread runs on regardless
    // and ends with the process.
    let _ = ready.send(Ok(()));
    let failure = pump();
    // A loop that ended without the window's destruction (an error, a
    // panic in the window procedure) destroys it here, which unregisters
    // what it registered for.
    if shared
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some()
    {
        destroy(hwnd);
    }
    unregister_class(class);
    Some(failure.or_else(|| FAILURE.take()))
}

/// Sends on the done channel when dropped: the thread's last act.
struct Finished(mpsc::Sender<()>);

impl Drop for Finished {
    fn drop(&mut self) {
        // The joiner may have stopped waiting.
        let _ = self.0.send(());
    }
}

/// A fresh class name for this thread's window.
fn class_name() -> Vec<u16> {
    let n = CLASSES.fetch_add(1, Ordering::Relaxed);
    wide(&format!(
        "auto-clicker.message-window.{}.{n}",
        std::process::id()
    ))
}

/// Registers `class` and creates its hidden top-level window.
#[allow(unsafe_code)]
fn create_window(class: &[u16]) -> io::Result<HWND> {
    // SAFETY: a null name asks for this program's own module, which needs
    // no freeing.
    let instance = unsafe { GetModuleHandleW(null()) };
    let window_class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(window_procedure),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: null_mut(),
        hCursor: null_mut(),
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class.as_ptr(),
    };
    // SAFETY: `window_class` is fully set, its name NUL-terminated UTF-16
    // that outlives the class (unregistered before `class` is dropped).
    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let title = wide("Auto Clicker");
    // SAFETY: the class was registered above with this module; the title
    // is NUL-terminated UTF-16 alive for the call. No parent, menu or
    // creation data. Without `WS_VISIBLE` the window is never shown, and a
    // tool window has no taskbar button should anything show it.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        let err = io::Error::last_os_error();
        unregister_class(class);
        return Err(err);
    }
    Ok(hwnd)
}

#[allow(unsafe_code)]
fn unregister_class(class: &[u16]) {
    // SAFETY: `class` is the NUL-terminated name registered with this
    // module; no window of it is left. A failure leaves only a class name,
    // freed with the process.
    unsafe { UnregisterClassW(class.as_ptr(), GetModuleHandleW(null())) };
}

/// Runs the message loop until `WM_QUIT`; the failure when `GetMessage`
/// fails instead.
#[allow(unsafe_code)]
fn pump() -> Option<String> {
    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is a live, writable `MSG`; no window or range
        // filter, so every message of this thread comes through.
        let got = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        match got {
            0 => return None,
            -1 => {
                return Some(format!(
                    "reading the message queue failed: {}",
                    io::Error::last_os_error()
                ));
            }
            _ => {
                // SAFETY: `message` was just filled in by `GetMessageW`.
                unsafe { DispatchMessageW(&message) };
            }
        }
    }
}

/// The window procedure. Everything it does runs inside `catch_unwind`,
/// since unwinding out of it would abort the process: a panic ends the
/// loop instead, and the thread reports it.
#[allow(unsafe_code)]
unsafe extern "system" fn window_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match catch_unwind(AssertUnwindSafe(|| answer(hwnd, message, wparam, lparam))) {
        Ok(Some(result)) => result,
        // SAFETY: the arguments are the ones Windows passed in.
        Ok(None) => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        Err(panic) => {
            FAILURE.set(Some(panicked(&*panic)));
            // SAFETY: no arguments but the exit code.
            unsafe { PostQuitMessage(1) };
            0
        }
    }
}

/// What the window answers to `message`; `None` leaves it to Windows'
/// default.
fn answer(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match message {
        WM_WTSSESSION_CHANGE => {
            if let Some(change) = SessionChange::from_code(wparam) {
                emit(WindowEvent::Session(change));
            }
            Some(0)
        }
        WM_POWERBROADCAST => {
            power(wparam, lparam);
            Some(1)
        }
        // Never holds up a logoff: the program releases on `WM_ENDSESSION`.
        WM_QUERYENDSESSION => Some(1),
        WM_ENDSESSION => {
            if wparam != 0 {
                emit(WindowEvent::EndSession);
            }
            Some(0)
        }
        // A request to close never destroys the window on its own: the
        // controller quits, and the thread is then told to.
        WM_CLOSE => {
            emit(WindowEvent::Close);
            Some(0)
        }
        WM_TIMER if wparam == RETRY_TIMER => {
            kill_retry_timer(hwnd);
            watch_session(hwnd);
            Some(0)
        }
        WM_QUIT_REQUEST => {
            destroy(hwnd);
            Some(0)
        }
        WM_DESTROY => {
            unwatch(hwnd);
            Some(0)
        }
        other => {
            if READY.get() {
                emit(WindowEvent::Other(other));
            }
            None
        }
    }
}

/// A `WM_POWERBROADCAST`.
fn power(event: WPARAM, lparam: LPARAM) {
    let Ok(event) = u32::try_from(event) else {
        return;
    };
    match event {
        PBT_APMSUSPEND => emit(WindowEvent::Suspend),
        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => emit(WindowEvent::Resume),
        PBT_POWERSETTINGCHANGE => {
            if let Some(on) = display_setting(lparam) {
                emit(WindowEvent::Display(on));
            }
        }
        _ => {}
    }
}

/// Whether the display is on, from a `PBT_POWERSETTINGCHANGE` notice's
/// setting; none for another setting or a malformed one.
#[allow(unsafe_code)]
fn display_setting(lparam: LPARAM) -> Option<bool> {
    let setting = lparam as *const POWERBROADCAST_SETTING;
    if setting.is_null() {
        return None;
    }
    // SAFETY: for `PBT_POWERSETTINGCHANGE` Windows passes a
    // `POWERBROADCAST_SETTING` that lives for the message's handling; only
    // its header is read here.
    let header = unsafe { &*setting };
    if !same_guid(&header.PowerSetting, &GUID_CONSOLE_DISPLAY_STATE)
        || usize::try_from(header.DataLength).map_or(true, |len| len < size_of::<u32>())
    {
        return None;
    }
    // SAFETY: `DataLength` says at least four bytes of data follow the
    // header, starting at `Data`; they may be unaligned.
    let state = unsafe { header.Data.as_ptr().cast::<u32>().read_unaligned() };
    Some(state != DISPLAY_OFF)
}

fn same_guid(a: &windows_sys::core::GUID, b: &windows_sys::core::GUID) -> bool {
    (a.data1, a.data2, a.data3, a.data4) == (b.data1, b.data2, b.data3, b.data4)
}

/// Registers for session notices, then reads the state once: a notice that
/// arrives meanwhile is queued, and handled after the state it follows. On
/// failure, tries again later.
#[allow(unsafe_code)]
fn watch_session(hwnd: HWND) {
    // SAFETY: `hwnd` is this thread's live window.
    if unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } == 0 {
        emit(WindowEvent::SessionUnwatched(io::Error::last_os_error()));
        schedule_retry(hwnd);
        return;
    }
    SESSION_WATCHED.set(true);
    RETRIES.set(0);
    emit(WindowEvent::SessionRead(query()));
}

/// Starts the retry timer for the session notices.
#[allow(unsafe_code)]
fn schedule_retry(hwnd: HWND) {
    let tries = RETRIES.get();
    RETRIES.set(tries.saturating_add(1));
    let delay = RETRY_DELAYS[tries.min(RETRY_DELAYS.len() - 1)];
    let millis = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: `hwnd` is this thread's live window; no callback, so the
    // timer arrives as `WM_TIMER`.
    if unsafe { SetTimer(hwnd, RETRY_TIMER, millis, None) } == 0 {
        emit(WindowEvent::SessionUnwatched(io::Error::last_os_error()));
    }
}

#[allow(unsafe_code)]
fn kill_retry_timer(hwnd: HWND) {
    // SAFETY: `hwnd` is this thread's window; killing a timer that is not
    // running only fails.
    unsafe { KillTimer(hwnd, RETRY_TIMER) };
}

/// Registers for the display's state; Windows then sends the current one.
#[allow(unsafe_code)]
fn watch_display(hwnd: HWND) {
    // SAFETY: `hwnd` is this thread's live window, and the GUID a static
    // constant.
    let registration = unsafe {
        RegisterPowerSettingNotification(
            hwnd,
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    };
    if registration == 0 {
        emit(WindowEvent::DisplayUnwatched(io::Error::last_os_error()));
    } else {
        DISPLAY.set(registration);
    }
}

/// Undoes every registration before the window goes, clears the shared
/// handle so nothing posts to it again, and ends the loop.
#[allow(unsafe_code)]
fn unwatch(hwnd: HWND) {
    READY.set(false);
    kill_retry_timer(hwnd);
    if SESSION_WATCHED.replace(false) {
        // SAFETY: `hwnd` is still alive (this runs in its `WM_DESTROY`) and
        // was registered. A failure leaves a registration that ends with
        // the window.
        unsafe { WTSUnRegisterSessionNotification(hwnd) };
    }
    let display = DISPLAY.replace(0);
    if display != 0 {
        // SAFETY: `display` came from a successful registration and is
        // unregistered once.
        unsafe { UnregisterPowerSettingNotification(display) };
    }
    SHARED.with_borrow(|shared| {
        if let Some(shared) = shared {
            *shared.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
        }
    });
    // SAFETY: no arguments but the exit code.
    unsafe { PostQuitMessage(0) };
}

#[allow(unsafe_code)]
fn destroy(hwnd: HWND) {
    // What Windows sends while it takes the window apart is not news.
    READY.set(false);
    // SAFETY: `hwnd` is this thread's window, destroyed on the thread that
    // created it; its `WM_DESTROY` undoes what it registered for.
    unsafe { DestroyWindow(hwnd) };
}

/// Hands `event` to the handler, after any that arrived while it was busy.
/// A message that arrives while the handler runs (only one sent by Windows
/// from inside a call the handler makes) waits its turn.
fn emit(event: WindowEvent) {
    PENDING.with_borrow_mut(|pending| pending.push_back(event));
    let Some(handler) = HANDLER.with_borrow_mut(Option::take) else {
        // The handler is busy further up this thread's stack, which drains
        // the queue once it returns; or the thread is ending.
        return;
    };
    let mut lent = Lent(Some(handler));
    while let Some(next) = PENDING.with_borrow_mut(VecDeque::pop_front) {
        if let Some(handler) = lent.0.as_mut() {
            handler(next);
        }
    }
}

/// The handler while it is called: put back when this is dropped, a panic
/// in it included, so the thread can still report its end.
struct Lent(Option<Handler>);

impl Drop for Lent {
    fn drop(&mut self) {
        HANDLER.set(self.0.take());
    }
}

/// A panic's message, as far as it has one.
fn panicked(panic: &(dyn Any + Send)) -> String {
    let what = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message");
    format!("the message thread panicked: {what}")
}

#[cfg(test)]
mod tests;
