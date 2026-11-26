//! The tray on Windows: what it shows, drawn from the controller's view on
//! the runtime each time the view changes, and handed to the message
//! thread, which shows it (map D1, M5 brief decision 15). The menu is
//! Linux's ([`tray_menu`]), labelled by the same texts; a choice comes back
//! as a command, which [`Commands`] turns into the controller's message.
//! Nothing here changes the mode.

use std::sync::{Arc, Mutex, PoisonError};

use auto_clicker_backend_windows::{MenuEntry, Remote, TrayContent, TrayIcon, icon_side};
use auto_clicker_core::controller::{Message, View};
use auto_clicker_core::locale::Language;
use auto_clicker_core::log;
use auto_clicker_core::recording::FileName;
use tokio::sync::watch;

use crate::tray_icon;
use crate::tray_menu::{Choice, Entry, entries};
use crate::tray_text;

/// What the Settings item says until the settings window comes to Windows
/// (WN10).
const SETTINGS_OFF: &str = "Settings (not on Windows yet)";

/// The menu's commands. Each item keeps its command for the daemon's life,
/// and so does each recording the menu ever listed, so a choice from a menu
/// opened before the view changed sends what its item said.
const START: u32 = 1;
const STOP: u32 = 2;
const RECORD: u32 = 3;
const STOP_RECORDING: u32 = 4;
const QUIT: u32 = 5;
/// The first recording's command; the next ones follow.
const FIRST_RECORDING: u32 = 100;

/// The recordings the menu has listed, by command: shared by the runtime,
/// which draws the menu, and the message thread, which hears the choice.
#[derive(Clone, Debug, Default)]
pub struct Commands(Arc<Mutex<Vec<FileName>>>);

impl Commands {
    /// The command of `message`, given one for a recording the first time
    /// it is listed; none for what the menu never sends.
    fn command(&self, message: &Message) -> Option<u32> {
        match message {
            Message::Start => Some(START),
            Message::Stop => Some(STOP),
            Message::Record => Some(RECORD),
            Message::StopRecording => Some(STOP_RECORDING),
            Message::Quit => Some(QUIT),
            Message::Play(file) => self.recording(file),
            _ => None,
        }
    }

    fn recording(&self, file: &FileName) -> Option<u32> {
        let mut listed = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let place = match listed.iter().position(|known| known == file) {
            Some(place) => place,
            None => {
                listed.push(file.clone());
                listed.len() - 1
            }
        };
        FIRST_RECORDING.checked_add(u32::try_from(place).ok()?)
    }

    /// The message a chosen `command` sends; none for one the menu never
    /// gave.
    pub fn message(&self, command: u32) -> Option<Message> {
        match command {
            START => Some(Message::Start),
            STOP => Some(Message::Stop),
            RECORD => Some(Message::Record),
            STOP_RECORDING => Some(Message::StopRecording),
            QUIT => Some(Message::Quit),
            _ => {
                let place = usize::try_from(command.checked_sub(FIRST_RECORDING)?).ok()?;
                let listed = self.0.lock().unwrap_or_else(PoisonError::into_inner);
                listed.get(place).cloned().map(Message::Play)
            }
        }
    }
}

/// Keeps the tray in step with the controller's view until the controller
/// or the message thread is gone. It sleeps while the view does not change.
pub async fn follow(
    mut views: watch::Receiver<View>,
    remote: Remote,
    language: Language,
    commands: Commands,
) {
    loop {
        let shown = content(&views.borrow_and_update(), language, icon_side(), &commands);
        if let Err(err) = remote.show_tray(shown) {
            if err.kind() == std::io::ErrorKind::NotConnected {
                return;
            }
            // Windows' queue was full: the next change tries again.
            log::line(format_args!(
                "auto-clicker: the tray was not updated: {err}"
            ));
        }
        if views.changed().await.is_err() {
            return;
        }
    }
}

/// What the tray shows for `view`, in `language`, with an icon `side`
/// pixels square.
pub fn content(view: &View, language: Language, side: u16, commands: &Commands) -> TrayContent {
    let image = tray_icon::image(view.activity, side);
    let mut menu: Vec<MenuEntry> = entries(view, false)
        .into_iter()
        .filter(|entry| entry.visible)
        .map(|entry| menu_entry(entry, language, commands))
        .collect();
    // Quit sits apart from the rest.
    menu.insert(menu.len().saturating_sub(1), MenuEntry::Separator);
    TrayContent {
        title: tray_text::title(view, language),
        details: tray_text::details(view, language),
        icon: TrayIcon {
            side: image.side,
            rgba: image.rgba,
        },
        menu,
    }
}

/// `entry` as the Windows menu shows it: a submenu with its shown items,
/// or an item with its command. Settings is off until WN10, and says so.
fn menu_entry(entry: Entry, language: Language, commands: &Commands) -> MenuEntry {
    let label = if entry.named {
        entry.label.clone()
    } else if entry.sends == Some(Choice::Settings) {
        tray_text::label(SETTINGS_OFF, language)
    } else {
        tray_text::label(&entry.label, language)
    };
    if !entry.children.is_empty() {
        let children = entry
            .children
            .into_iter()
            .filter(|child| child.visible)
            .map(|child| menu_entry(child, language, commands))
            .collect();
        return MenuEntry::Submenu { label, children };
    }
    let command = match &entry.sends {
        Some(Choice::Send(message)) => commands.command(message),
        Some(Choice::Settings) | None => None,
    };
    MenuEntry::Item {
        label,
        enabled: entry.enabled && command.is_some(),
        command,
        checked: entry.checked,
    }
}

#[cfg(test)]
mod tests;
