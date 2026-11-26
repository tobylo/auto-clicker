use std::collections::VecDeque;

use super::*;
use crate::TrayIcon;

/// A taskbar that answers as told: each call takes the next answer for its
/// kind (none left: it succeeds), and every call is written down.
#[derive(Default)]
struct Fake {
    calls: Vec<String>,
    adds: VecDeque<bool>,
    modifies: VecDeque<bool>,
    no_taskbar: bool,
    told: Vec<String>,
}

fn answer(answers: &mut VecDeque<bool>) -> io::Result<()> {
    if answers.pop_front().unwrap_or(true) {
        Ok(())
    } else {
        Err(io::Error::other("refused"))
    }
}

impl Calls for Fake {
    fn add(&mut self, shown: &Shown) -> io::Result<()> {
        self.calls.push(format!("add {}", shown.title));
        answer(&mut self.adds)
    }

    fn modify(&mut self, shown: &Shown) -> io::Result<()> {
        self.calls.push(format!("modify {}", shown.title));
        answer(&mut self.modifies)
    }

    fn version(&mut self) -> io::Result<()> {
        self.calls.push("version".to_owned());
        Ok(())
    }

    fn delete(&mut self) {
        self.calls.push("delete".to_owned());
    }

    fn taskbar_present(&mut self) -> bool {
        !self.no_taskbar
    }

    fn tell(&mut self, event: WindowEvent) {
        self.told.push(match event {
            WindowEvent::TrayShown => "shown".to_owned(),
            WindowEvent::TrayUnshown(err) => format!("unshown: {err}"),
            other => format!("{other:?}"),
        });
    }
}

fn shown(title: &str) -> Inbox {
    Inbox {
        show: Some(Shown {
            title: title.to_owned(),
            icon: TrayIcon {
                side: 1,
                rgba: vec![0; 4],
            },
        }),
        ..Inbox::default()
    }
}

fn restarted() -> Inbox {
    Inbox {
        restarted: true,
        ..Inbox::default()
    }
}

/// The calls made, and what the handler was told, since the last look.
fn seen(icon: &mut Icon<Fake>) -> (Vec<String>, Vec<String>) {
    (
        std::mem::take(&mut icon.calls.calls),
        std::mem::take(&mut icon.calls.told),
    )
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn the_first_picture_adds_the_icon_and_the_next_change_it() {
    let mut icon = Icon::new(Fake::default());
    let now = Instant::now();
    // Nothing to show yet: nothing is added.
    icon.take(restarted(), now);
    assert_eq!(seen(&mut icon), (vec![], vec![]));
    icon.take(shown("idle"), now);
    icon.take(shown("starts in 3 s"), now);
    // A look with nothing new changes nothing.
    icon.take(Inbox::default(), now);
    assert_eq!(
        seen(&mut icon),
        (strings(&["add idle", "modify starts in 3 s"]), vec![])
    );
    icon.remove();
    assert_eq!(seen(&mut icon).0, strings(&["delete"]));
}

#[test]
fn a_refused_icon_is_tried_again_on_the_ladder_change_first() {
    let mut icon = Icon::new(Fake {
        adds: VecDeque::from([false, false, false]),
        modifies: VecDeque::from([false, false]),
        ..Fake::default()
    });
    let start = Instant::now();
    icon.take(shown("idle"), start);
    assert_eq!(
        seen(&mut icon),
        (strings(&["add idle"]), strings(&["unshown: refused"]))
    );
    assert_eq!(
        icon.state,
        State::Retrying {
            due: start + RETRY_DELAYS[0]
        }
    );
    // New content while it waits is kept, and shown at the try.
    icon.take(shown("clicking"), start);
    assert_eq!(seen(&mut icon), (vec![], vec![]));
    // Not due yet: nothing.
    icon.take(Inbox::default(), start + RETRY_DELAYS[0] / 2);
    assert_eq!(seen(&mut icon), (vec![], vec![]));
    // Due: a change first, since a refused call may have gone through,
    // then an add; refused again, the next try waits longer.
    let first = start + RETRY_DELAYS[0];
    icon.take(Inbox::default(), first);
    assert_eq!(
        seen(&mut icon),
        (
            strings(&["modify clicking", "add clicking"]),
            strings(&["unshown: refused"])
        )
    );
    assert_eq!(
        icon.state,
        State::Retrying {
            due: first + RETRY_DELAYS[1]
        }
    );
    // The change goes through: it shows, and its version is set.
    let second = first + RETRY_DELAYS[1];
    icon.take(Inbox::default(), second);
    icon.take(Inbox::default(), second);
    assert_eq!(
        seen(&mut icon).0,
        strings(&["modify clicking", "add clicking"])
    );
    let third = second + RETRY_DELAYS[2];
    icon.take(Inbox::default(), third);
    assert_eq!(
        seen(&mut icon),
        (
            strings(&["modify clicking", "version"]),
            strings(&["shown"])
        )
    );
    assert_eq!((icon.state, icon.tries), (State::Shown, 0));
}

#[test]
fn the_ladder_stops_lengthening_at_its_last_step() {
    let mut icon = Icon::new(Fake {
        adds: VecDeque::from([false; 20]),
        modifies: VecDeque::from([false; 20]),
        ..Fake::default()
    });
    let mut now = Instant::now();
    icon.take(shown("idle"), now);
    for _ in 0..12 {
        let State::Retrying { due } = icon.state else {
            panic!("{:?}", icon.state);
        };
        assert!(due - now <= *RETRY_DELAYS.last().unwrap());
        now = due;
        icon.take(Inbox::default(), now);
    }
    let State::Retrying { due } = icon.state else {
        panic!("{:?}", icon.state);
    };
    assert_eq!(due - now, *RETRY_DELAYS.last().unwrap());
}

#[test]
fn with_no_taskbar_the_icon_waits_for_explorer_and_comes_back_with_it() {
    let mut icon = Icon::new(Fake {
        adds: VecDeque::from([false]),
        no_taskbar: true,
        ..Fake::default()
    });
    let now = Instant::now();
    icon.take(shown("idle"), now);
    assert_eq!(icon.state, State::Waiting);
    // No try is due, and new content waits too.
    icon.take(shown("clicking"), now + Duration::from_secs(3600));
    seen(&mut icon);
    icon.take(restarted(), now);
    assert_eq!(
        seen(&mut icon),
        (strings(&["add clicking"]), strings(&["shown"]))
    );
    // Explorer restarts again: the shown icon went with it, and is added
    // afresh, with no news for the handler.
    icon.take(restarted(), now);
    assert_eq!(seen(&mut icon), (strings(&["add clicking"]), vec![]));
}

#[test]
fn an_icon_never_added_or_gone_with_explorer_is_not_removed() {
    let mut never = Icon::new(Fake::default());
    never.remove();
    assert_eq!(seen(&mut never).0, Vec::<String>::new());
    // A refused add may have gone through: it is removed.
    let mut refused = Icon::new(Fake {
        adds: VecDeque::from([false]),
        ..Fake::default()
    });
    refused.take(shown("idle"), Instant::now());
    seen(&mut refused);
    refused.remove();
    assert_eq!(seen(&mut refused).0, strings(&["delete"]));
}

/// A fake whose calls are counted where the test can see them while the
/// thread runs.
struct Counted(Arc<Mutex<Vec<String>>>);

impl Calls for Counted {
    fn add(&mut self, shown: &Shown) -> io::Result<()> {
        self.note(format!("add {}", shown.title));
        Ok(())
    }

    fn modify(&mut self, shown: &Shown) -> io::Result<()> {
        self.note(format!("modify {}", shown.title));
        Ok(())
    }

    fn version(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn delete(&mut self) {
        self.note("delete".to_owned());
    }

    fn taskbar_present(&mut self) -> bool {
        true
    }

    fn tell(&mut self, _: WindowEvent) {}
}

impl Counted {
    fn note(&self, call: String) {
        self.0.lock().unwrap().push(call);
    }
}

#[test]
fn the_thread_shows_the_latest_and_removes_the_icon_on_its_way_out() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let shell = Shell::spawn(Counted(Arc::clone(&calls))).unwrap();
    shell.show(shown("idle").show.unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while calls.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "nothing shown");
        thread::sleep(Duration::from_millis(10));
    }
    let asked = Instant::now();
    shell.remove(Duration::from_secs(10));
    assert!(asked.elapsed() < Duration::from_secs(5));
    assert_eq!(*calls.lock().unwrap(), strings(&["add idle", "delete"]));
}

/// A taskbar that never answers, as a hung Explorer does.
struct Hung;

impl Calls for Hung {
    fn add(&mut self, _: &Shown) -> io::Result<()> {
        thread::sleep(Duration::from_secs(3600));
        Ok(())
    }

    fn modify(&mut self, _: &Shown) -> io::Result<()> {
        Ok(())
    }

    fn version(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn delete(&mut self) {}

    fn taskbar_present(&mut self) -> bool {
        true
    }

    fn tell(&mut self, _: WindowEvent) {}
}

#[test]
fn a_hung_explorer_holds_up_neither_a_change_nor_the_removal_past_its_deadline() {
    let shell = Shell::spawn(Hung).unwrap();
    let asked = Instant::now();
    shell.show(shown("idle").show.unwrap());
    shell.show(shown("clicking").show.unwrap());
    let within = Duration::from_millis(200);
    shell.remove(within);
    // The thread is left in its call, to end with the process.
    assert!(asked.elapsed() < Duration::from_secs(2));
}
