//! Releases Windows blocked, sent again by the watchdog (WN06b).

use super::*;

/// What the watchdog logs when its first try left releases owed.
const STAYS: &str = "maybe not delivered; sending them again";
/// What it logs once it delivered them.
const DELIVERED_LATER: &str = "releases left are delivered";
/// How long a watchdog may take to deliver once the release can arrive:
/// an event, then a pass.
const DELIVERED: Duration = Duration::from_secs(5);

/// Waits until the watchdog's log in `home` says `what`, at most `within`.
fn watchdog_said(home: &Home, what: &str, within: Duration) {
    let deadline = Instant::now() + within;
    while !home.watchdog_log().contains(what) {
        assert!(
            Instant::now() < deadline,
            "the watchdog did not say {what:?}: {}",
            home.watchdog_log()
        );
        sleep(Duration::from_millis(20));
    }
}

/// Starts an unelevated daemon of `home`'s, as `what`, and its hold of F13
/// from the tray.
fn holding_unelevated(home: &Home, what: &str) -> (Unelevated, desktop::Process) {
    home.config(HOLD_F13);
    let (daemon, pid) = Unelevated::start(home, what);
    let icon = promoted_icon(pid);
    choose(icon, None, label("Start clicking"));
    wait_for("the hold", || desktop::is_down(VK_F13).then_some(()));
    (daemon, pid)
}

/// Kills `daemon`'s daemon while the test's elevated window is in front,
/// as Task Manager would, and checks its watchdog stays to send the
/// release again; the window, still in front.
fn killed_behind_an_elevated_window(home: &Home, daemon: &Unelevated) -> desktop::Front {
    let front = in_front(false);
    assert!(kill(daemon.daemon.unwrap(), false).success());
    watchdog_said(home, STAYS, END);
    // Nothing can deliver it while the elevated window is in front.
    sleep(Duration::from_secs(1));
    assert!(alive(daemon.watchdog.unwrap()), "{}", home.watchdog_log());
    assert!(!home.watchdog_log().contains(DELIVERED_LATER));
    front
}

#[test]
#[ignore = "must run elevated, in the desktop: the WN01 VM through `vm run --elevated`"]
fn a_release_an_elevated_window_blocked_is_sent_again_once_it_leaves() {
    if !auto_clicker_backend_windows::is_elevated().unwrap() {
        println!("not elevated: run this test alone through `vm run --elevated`");
        return;
    }
    let home = Home::new("elevated-again");
    let (daemon, _) = holding_unelevated(&home, "again");
    let front = killed_behind_an_elevated_window(&home, &daemon);
    let leaving = Instant::now();
    drop(front);
    watchdog_said(&home, DELIVERED_LATER, DELIVERED);
    println!("delivered {:?} after the window left", leaving.elapsed());
    assert!(!desktop::is_down(VK_F13));
    wait_for_end(daemon.watchdog.unwrap(), END);
    assert!(!alive(daemon.watchdog.unwrap()), "{}", home.watchdog_log());
    drop(daemon);
}

/// What a daemon's tray says while an earlier watchdog still sends.
fn earlier_said(text: &str) -> bool {
    text.contains("an earlier Auto Clicker is still letting go of held keys")
        || text.contains("ett tidigare Auto Clicker släpper fortfarande nedtryckta tangenter")
}

#[test]
#[ignore = "must run elevated, in the desktop: the WN01 VM through `vm run --elevated`"]
fn a_new_start_refuses_starts_while_an_earlier_watchdog_still_sends() {
    if !auto_clicker_backend_windows::is_elevated().unwrap() {
        println!("not elevated: run this test alone through `vm run --elevated`");
        return;
    }
    let home = Home::new("earlier");
    let (daemon, _) = holding_unelevated(&home, "earlier");
    let front = killed_behind_an_elevated_window(&home, &daemon);
    // A new start meanwhile, as a sign-in or the user's would be.
    let later = Home::new("later");
    later.config(HOLD_F13);
    let (second, pid) = Unelevated::start(&later, "later");
    let icon = promoted_icon(pid);
    println!("{}", popup_once(pid, icon, earlier_said));
    drop(front);
    watchdog_said(&home, DELIVERED_LATER, DELIVERED);
    wait_for_end(daemon.watchdog.unwrap(), END);
    // The earlier watchdog let go of its mutex: the new daemon allows starts.
    wait_for("the new daemon to hear it", || {
        later.log().contains("EarlierWatchdogDone").then_some(())
    });
    popup_once(pid, icon, |text| !earlier_said(text));
    choose(icon, None, label("Start clicking"));
    wait_for("the new hold", || desktop::is_down(VK_F13).then_some(()));
    drop(second);
    drop(daemon);
    // The new daemon's watchdog released the hold when it was killed.
    wait_for("F13 up", || (!desktop::is_down(VK_F13)).then_some(()));
}

#[test]
#[ignore = "locks the session and waits for it to be unlocked by hand: run alone in the WN01 VM through `vm run`"]
fn a_release_the_lock_screen_blocked_is_sent_again_after_unlock() {
    if std::env::var_os("AUTO_CLICKER_TEST_UNLOCK").is_none() {
        println!("AUTO_CLICKER_TEST_UNLOCK is not set: nobody would unlock");
        return;
    }
    let (home, mut daemon) = holding("lock-again", HOLD_F13, VK_F13);
    // The release is to reach this window: in front again after unlock.
    let front = in_front(false);
    let ups = desktop::Front::key_ups(VK_F13);
    desktop::lock().unwrap();
    wait_for("the lock", || {
        (!auto_clicker_backend_windows::foreground_takes_input()).then_some(())
    });
    // The lock stops the hold; its release cannot reach the lock screen's
    // desktop, so it stays owed, and the daemon's watchdog sends it.
    sleep(Duration::from_secs(1));
    assert!(kill(daemon.id(), false).success());
    watchdog_said(&home, STAYS, END);
    assert_eq!(desktop::Front::key_ups(VK_F13), ups, "a release arrived");
    println!("locked: unlock the session through the QEMU monitor");
    let deadline = Instant::now() + Duration::from_secs(180);
    while !auto_clicker_backend_windows::foreground_takes_input() {
        assert!(Instant::now() < deadline, "not unlocked within 3 minutes");
        sleep(Duration::from_millis(50));
    }
    let unlocked = Instant::now();
    watchdog_said(&home, DELIVERED_LATER, DELIVERED);
    println!("delivered {:?} after the unlock", unlocked.elapsed());
    wait_for("the release in the window", || {
        (desktop::Front::key_ups(VK_F13) > ups).then_some(())
    });
    assert_eq!(ended(&mut daemon.watchdog, END).code(), Some(1));
    drop(front);
}
