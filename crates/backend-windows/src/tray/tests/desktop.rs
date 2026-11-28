//! The tray in a real desktop: the taskbar, the pointer and the menu, in
//! the WN01 VM. Each test brings its own icon on its own message thread;
//! they share the screen, so they run one at a time.

use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows_sys::Win32::UI::WindowsAndMessaging::WM_WTSSESSION_CHANGE;
use windows_sys::Win32::UI::WindowsAndMessaging::WTS_SESSION_LOCK;

use super::super::*;
use super::content;
use crate::desktop::{self, Area, Button, Line};
use crate::message_thread::tests::{ARRIVAL, started};
use crate::{MessageThread, SessionChange};

/// How long a lock posted while the menu is open may take to reach the
/// handler: far above what it takes, far below a second.
const NOTICE: Duration = Duration::from_millis(500);
/// How long a quit with the menu open may take, the daemon's own deadline.
const QUIT: Duration = Duration::from_secs(1);

/// Waits for `what` to give something, at most [`ARRIVAL`].
fn wait_for<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + ARRIVAL;
    loop {
        if let Some(found) = ready() {
            return found;
        }
        assert!(Instant::now() < deadline, "no {what} in {ARRIVAL:?}");
        sleep(Duration::from_millis(50));
    }
}

/// How long a PowerShell step may take: generous, and far under the
/// runner's deadline, so a stalled one still lets Explorer be resumed.
const POWERSHELL: Duration = Duration::from_secs(60);

fn powershell(script: &str) -> String {
    desktop::powershell(script, POWERSHELL).unwrap()
}

/// Brings the icons of `exe` out of the taskbar's overflow, where Windows
/// 11 puts a new icon, as a user does once in the taskbar's settings.
pub fn promote(exe: &Path) {
    let script = format!(
        "$n = 0; Get-ChildItem 'HKCU:\\Control Panel\\NotifyIconSettings' | ForEach-Object {{ \
         $p = Get-ItemProperty $_.PSPath; if ($p.ExecutablePath -eq '{}') {{ \
         Set-ItemProperty $_.PSPath -Name IsPromoted -Value 1 -Type DWord; $n++ }} }}; $n",
        // Quoted for PowerShell: an apostrophe is doubled.
        exe.display().to_string().replace('\'', "''")
    );
    let promoted = wait_for("entry in the taskbar's settings", || {
        let count: u32 = powershell(&script).trim().parse().unwrap_or(0);
        (count > 0).then_some(count)
    });
    assert!(promoted > 0);
}

/// A message thread that is ended however the test ends, a failed
/// assertion included: one left running would leave its icon for the next
/// test to find.
struct Running(Option<MessageThread>);

impl Running {
    fn remote(&self) -> crate::Remote {
        self.0.as_ref().map(MessageThread::remote).unwrap()
    }

    /// Ends the thread within `within`.
    fn quit_and_join(mut self, within: Duration) -> Result<(), crate::JoinError> {
        self.0.take().unwrap().quit_and_join(within)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(thread) = self.0.take() {
            let _ = thread.quit_and_join(ARRIVAL);
        }
    }
}

/// A thread showing the test content, its icon out of the overflow, and
/// where the icon is.
fn shown() -> (Running, std::sync::mpsc::Receiver<WindowEvent>, Area) {
    let (thread, received) = started();
    let thread = Running(Some(thread));
    thread
        .remote()
        .show_tray(content("Auto clicker: idle"))
        .unwrap();
    let pid = desktop::Process::this();
    wait_for("icon", || desktop::tray_icon(pid));
    promote(&std::env::current_exe().unwrap());
    // Promoted, it leaves the overflow's button for a place of its own.
    sleep(Duration::from_secs(1));
    let icon = wait_for("icon", || desktop::tray_icon(pid));
    (thread, received, icon)
}

/// Opens the menu with a click on `icon`; its lines.
fn open_menu(icon: Area, button: Button) -> desktop::Menu {
    desktop::click(icon.centre(), button).unwrap();
    wait_for("menu", desktop::shown_menu)
}

fn labels(lines: &[Line]) -> Vec<(String, bool)> {
    lines
        .iter()
        .map(|line| (line.label.clone(), line.greyed))
        .collect()
}

/// The next event that is not Windows' own, nor the icon's coming and
/// going (a first add the taskbar refused and a later try that took it),
/// within [`ARRIVAL`].
fn next_tray_event(received: &std::sync::mpsc::Receiver<WindowEvent>) -> WindowEvent {
    let deadline = Instant::now() + ARRIVAL;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match received.recv_timeout(left).unwrap() {
            WindowEvent::Other(_)
            | WindowEvent::SessionRead(_)
            | WindowEvent::Display(_)
            | WindowEvent::TrayUnshown(_)
            | WindowEvent::TrayShown => {}
            event => return event,
        }
    }
}

#[test]
#[ignore = "needs the taskbar and restarts Explorer: run in the WN01 VM through `vm run`"]
fn the_icon_comes_back_when_explorer_restarts() {
    let (thread, received, _) = shown();
    let pid = desktop::Process::this();
    powershell("Stop-Process -Name explorer -Force");
    wait_for("icon to go with Explorer", || {
        desktop::tray_icon(pid).is_none().then_some(())
    });
    // Windows starts Explorer again on its own, else the test does; the
    // icon is added afresh when the new one says so, with the content
    // shown last.
    sleep(Duration::from_secs(3));
    powershell(
        "if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer }",
    );
    wait_for("icon after the restart", || {
        sleep(Duration::from_millis(200));
        desktop::tray_icon(pid)
    });
    // The new taskbar has the icon because the thread added it again.
    assert!(
        !received
            .try_iter()
            .any(|event| matches!(event, WindowEvent::Ended { .. })),
    );
    let owner = desktop::icon_owner(pid).unwrap();
    thread.quit_and_join(ARRIVAL).unwrap();
    assert_eq!(owner.area(), None, "the icon outlived its window");
}

#[test]
#[ignore = "needs the taskbar and moves the pointer: run in the WN01 VM through `vm run`"]
fn the_popup_shows_every_line_while_the_pointer_rests_on_the_icon() {
    let (thread, _received, icon) = shown();
    let pid = desktop::Process::this();
    let on_icon = icon.centre();
    desktop::move_pointer(on_icon).unwrap();
    let text = wait_for("popup", || desktop::popup_text(pid));
    assert_eq!(text, "Auto clicker: idle\nA first line\nRock & roll");
    // A click on it reaches the window below, from any thread.
    assert_eq!(desktop::popup_takes_clicks(pid), Some(false));
    // New content shows while it is open.
    let mut counting = content("Auto clicker: starts in 3 s");
    counting.details.push("A third line".to_owned());
    thread.remote().show_tray(counting).unwrap();
    wait_for("new lines", || {
        desktop::popup_text(pid).filter(|text| text.ends_with("A third line"))
    });
    desktop::move_pointer(on_icon.down(-300)).unwrap();
    wait_for("popup to hide", || {
        desktop::popup_text(pid).is_none().then_some(())
    });
    thread.quit_and_join(ARRIVAL).unwrap();
}

#[test]
#[ignore = "needs the taskbar and clicks: run in the WN01 VM through `vm run`"]
fn the_menu_shows_its_entries_and_reports_the_choice() {
    let (thread, received, icon) = shown();
    let menu = open_menu(icon, Button::Left);
    assert!(matches!(
        next_tray_event(&received),
        WindowEvent::TrayMenuOpening
    ));
    let lines = menu.lines();
    assert_eq!(
        labels(&lines),
        [
            // An ampersand in a label shows as one: never an access key.
            ("Start clicking".to_owned(), false),
            ("Play".to_owned(), false),
            ("Record".to_owned(), true),
            (String::new(), true),
            ("Quit".to_owned(), false),
        ]
    );
    assert!(lines[1].submenu && lines[3].separator);
    menu.click(0).unwrap();
    assert!(matches!(
        next_tray_event(&received),
        WindowEvent::TrayChosen(1)
    ));
    // A right click opens it too; the Play submenu holds its recording,
    // checked, and choosing it reports its command.
    sleep(Duration::from_secs(1));
    let menu = open_menu(icon, Button::Right);
    assert!(matches!(
        next_tray_event(&received),
        WindowEvent::TrayMenuOpening
    ));
    desktop::move_pointer(menu.area(1).unwrap().centre()).unwrap();
    let play = wait_for("Play submenu", || {
        desktop::shown_menu().filter(|shown| {
            shown
                .lines()
                .first()
                .is_some_and(|line| line.label == "Rock && roll")
        })
    });
    assert!(play.lines()[0].checked);
    play.click(0).unwrap();
    assert!(matches!(
        next_tray_event(&received),
        WindowEvent::TrayChosen(100)
    ));
    thread.quit_and_join(ARRIVAL).unwrap();
}

#[test]
#[ignore = "needs the taskbar and clicks: run in the WN01 VM through `vm run`"]
fn a_click_on_the_icon_that_closes_the_menu_does_not_open_it_again() {
    let (thread, received, icon) = shown();
    open_menu(icon, Button::Left);
    desktop::click(icon.centre(), Button::Left).unwrap();
    wait_for("menu to close", || {
        desktop::shown_menu().is_none().then_some(())
    });
    // Long enough for the click's report to arrive, and open it again.
    sleep(Duration::from_secs(1));
    assert!(desktop::shown_menu().is_none(), "the menu opened again");
    let opened = received
        .try_iter()
        .filter(|event| matches!(event, WindowEvent::TrayMenuOpening))
        .count();
    assert_eq!(opened, 1);
    // A later click opens it.
    open_menu(icon, Button::Left);
    desktop::press(VK_ESCAPE).unwrap();
    thread.quit_and_join(ARRIVAL).unwrap();
}

#[test]
#[ignore = "needs the taskbar and clicks: run in the WN01 VM through `vm run`"]
fn a_lock_while_the_menu_is_open_reaches_the_handler_at_once() {
    let (thread, received, icon) = shown();
    open_menu(icon, Button::Right);
    received.try_iter().for_each(drop);
    let posted = Instant::now();
    thread
        .remote()
        .post(WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK as usize, 0)
        .unwrap();
    loop {
        let left = NOTICE.saturating_sub(posted.elapsed());
        match received.recv_timeout(left) {
            Ok(WindowEvent::Session(SessionChange::Lock)) => break,
            Ok(_) => {}
            Err(err) => panic!("no lock within {NOTICE:?}: {err}"),
        }
    }
    assert!(desktop::shown_menu().is_some(), "the menu closed");
    desktop::press(VK_ESCAPE).unwrap();
    thread.quit_and_join(ARRIVAL).unwrap();
}

#[test]
#[ignore = "needs the taskbar and clicks: run in the WN01 VM through `vm run`"]
fn a_quit_with_the_menu_open_closes_it_and_ends_the_thread_in_time() {
    let (thread, received, icon) = shown();
    open_menu(icon, Button::Left);
    let owner = desktop::icon_owner(desktop::Process::this()).unwrap();
    let asked = Instant::now();
    thread.quit_and_join(QUIT).unwrap();
    assert!(asked.elapsed() < QUIT);
    assert!(
        desktop::shown_menu().is_none(),
        "the menu outlived the thread"
    );
    assert_eq!(owner.area(), None, "the icon outlived the thread");
    // Nothing was chosen on the way out.
    assert!(
        !received
            .try_iter()
            .any(|event| matches!(event, WindowEvent::TrayChosen(_)))
    );
}

/// Explorer suspended, as a hung one is: resumed when dropped, however the
/// test ends.
struct Suspended;

/// Suspends or resumes every Explorer process through ntdll, each tried
/// even when one fails; whether every call succeeded.
fn explorer(call: &str) -> bool {
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; \
         public static class Nt {{ [DllImport(\"ntdll.dll\")] public static extern int \
         NtSuspendProcess(IntPtr h); [DllImport(\"ntdll.dll\")] public static extern int \
         NtResumeProcess(IntPtr h); }}'; \
         $all = @(Get-Process explorer); \
         if ($all.Count -eq 0) {{ exit 1 }}; $failed = 0; $all | ForEach-Object {{ \
         try {{ if ([Nt]::{call}($_.Handle) -ne 0) {{ $failed++ }} }} \
         catch {{ $failed++ }} }}; exit $failed"
    );
    desktop::powershell(&script, POWERSHELL).is_ok()
}

impl Suspended {
    fn explorer() -> Suspended {
        // Made first, so a suspension that fails part way is undone.
        let suspended = Suspended;
        assert!(explorer("NtSuspendProcess"), "Explorer was not suspended");
        suspended
    }
}

impl Drop for Suspended {
    fn drop(&mut self) {
        let resumed = explorer("NtResumeProcess");
        assert!(
            resumed || std::thread::panicking(),
            "Explorer was not resumed"
        );
    }
}

#[test]
#[ignore = "needs the taskbar and suspends Explorer: run in the WN01 VM through `vm run`"]
fn a_hung_explorer_holds_up_neither_a_lock_nor_the_quit() {
    let (thread, received, _) = shown();
    let hung = Suspended::explorer();
    // The change waits on Explorer, in the shell thread alone.
    thread
        .remote()
        .show_tray(content("Auto clicker: clicking"))
        .unwrap();
    sleep(Duration::from_millis(200));
    received.try_iter().for_each(drop);
    let posted = Instant::now();
    thread
        .remote()
        .post(WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK as usize, 0)
        .unwrap();
    loop {
        let left = NOTICE.saturating_sub(posted.elapsed());
        match received.recv_timeout(left) {
            Ok(WindowEvent::Session(SessionChange::Lock)) => break,
            Ok(_) => {}
            Err(err) => panic!("no lock within {NOTICE:?}: {err}"),
        }
    }
    let asked = Instant::now();
    thread.quit_and_join(QUIT).unwrap();
    assert!(asked.elapsed() < QUIT);
    drop(hung);
}
