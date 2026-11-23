//! The group "On the screen" on Slint's software renderer, in Swedish: it
//! shows the four switches and the corner the daemon sent, saves what they
//! hold, and is greyed out with the reason where the display cannot show,
//! whatever the switches say.

use auto_clicker_core::config::{Corner, OnScreenSettings};
use auto_clicker_core::message::NoOnScreen;
use auto_clicker_core::window::{Ask, Update, WindowView};

use super::list::{opened, sent};
use crate::state::tests::{defaults, view};
use crate::ui::SettingsWindow;

/// The group's switches and corner as the window holds them.
fn shown(window: &SettingsWindow) -> (bool, bool, bool, bool, i32) {
    (
        window.get_show_countdown(),
        window.get_badge_clicking(),
        window.get_badge_recording(),
        window.get_badge_playback(),
        window.get_badge_corner(),
    )
}

pub fn the_group_shows_the_switches_saves_them_and_says_why_it_cannot_show() {
    let (window, app, mut read) = opened();
    let update = |shown: WindowView| app.borrow_mut().update(Update::View(Box::new(shown)));
    let mut settings = defaults();
    settings.on_screen = OnScreenSettings {
        countdown: true,
        clicking: false,
        recording: true,
        playback: false,
        corner: Corner::BottomLeft,
    };
    update(view(settings.clone(), None));
    assert_eq!(shown(&window), (true, false, true, false, 2));
    assert!(window.get_on_screen_possible());
    assert_eq!(window.get_on_screen_why(), "");

    // What the fields hold is what is saved.
    window.set_show_countdown(false);
    window.set_badge_playback(true);
    window.set_badge_corner(1);
    window.invoke_save();
    let Ask::Save {
        settings: saved, ..
    } = sent(&mut read).ask
    else {
        panic!("not a save");
    };
    let expected = OnScreenSettings {
        countdown: false,
        clicking: false,
        recording: true,
        playback: true,
        corner: Corner::TopRight,
    };
    assert_eq!(saved.on_screen, expected);

    // A desktop without layer-shell: greyed out with the reason, the
    // switches all off included, and the values kept for a save.
    for on_screen in [settings.on_screen, OnScreenSettings::default(), all_off()] {
        let (window, app, _read) = opened();
        let mut lacking = view(defaults(), None);
        lacking.settings.on_screen = on_screen;
        lacking.no_on_screen = Some(NoOnScreen::NoLayerShell);
        app.borrow_mut().update(Update::View(Box::new(lacking)));
        assert!(!window.get_on_screen_possible(), "{on_screen:?}");
        assert_eq!(
            window.get_on_screen_why(),
            "skrivbordet kan inte visa den (inget layer-shell)"
        );
        let corner = Corner::ALL.iter().position(|c| *c == on_screen.corner);
        assert_eq!(usize::try_from(shown(&window).4).ok(), corner);
        // Shown again once it can.
        let mut able = view(defaults(), None);
        able.settings.on_screen = on_screen;
        app.borrow_mut().update(Update::View(Box::new(able)));
        assert!(window.get_on_screen_possible(), "{on_screen:?}");
        assert_eq!(window.get_on_screen_why(), "");
    }
}

fn all_off() -> OnScreenSettings {
    OnScreenSettings {
        countdown: false,
        clicking: false,
        recording: false,
        playback: false,
        corner: Corner::TopRight,
    }
}
