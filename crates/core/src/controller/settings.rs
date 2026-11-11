//! The settings window as the controller sees it: a link that brings the
//! window's requests and takes the view it shows, and the config file the
//! window's saves go to.
//!
//! The controller checks and writes a save, and takes what it can into use
//! at once: the clicker, playback and tray settings from the next start
//! (never during a run), the hotkeys as soon as it is idle, when it starts
//! the reader again with them. `[input]` picks the platform, so it applies
//! from the daemon's next start.

use std::future::{Future, poll_fn};
use std::task::Poll;

use tokio::sync::{mpsc, watch};

use super::hotkeys::HotkeySource;
use super::{Controller, InputReader, Platform, State};
use crate::config::{Config, ConfigFile, Draft, Field, Refusal, SaveError, check};
use crate::hotkeys::{Action, Bindings, Modifiers};
use crate::window::{Answer, Ask, Confirm, Outcome, Request, ShownHotkeys, WindowView};

/// A settings window's link to the controller. The controller drops it to
/// end the window's session; the window's side ends it by closing
/// `requests`.
pub struct WindowLink {
    /// What the window asks.
    pub requests: mpsc::Receiver<Request>,
    /// What it shows: `None` until the controller published the first view.
    pub view: watch::Sender<Option<WindowView>>,
}

/// Where a new window's link goes. The controller serves one window at a
/// time: a new link ends the previous one's session.
pub type WindowOpener = mpsc::UnboundedSender<WindowLink>;

/// The config as saved, and the window that edits it.
pub(super) struct Settings {
    /// The settings in the config file: what the window shows. The
    /// controller's own `config` is what it runs with.
    saved: Config,
    file: ConfigFile,
    opener: WindowOpener,
    openings: mpsc::UnboundedReceiver<WindowLink>,
    window: Option<WindowLink>,
    /// The answer to the window's last request.
    answer: Option<Answer>,
}

/// What happened with the settings window.
pub(super) enum WindowEvent {
    Opened(WindowLink),
    Request(Request),
    /// The window closed its side, or crashed.
    Closed,
}

impl Settings {
    pub(super) fn new(saved: Config) -> Settings {
        let (opener, openings) = mpsc::unbounded_channel();
        Settings {
            saved,
            file: ConfigFile::none(),
            opener,
            openings,
            window: None,
            answer: None,
        }
    }
}

/// The next thing a window did; never resolves while none is open and none
/// is opening.
pub(super) fn next_event(settings: &mut Settings) -> impl Future<Output = WindowEvent> + '_ {
    poll_fn(move |cx| {
        if let Poll::Ready(Some(link)) = settings.openings.poll_recv(cx) {
            return Poll::Ready(WindowEvent::Opened(link));
        }
        let Some(window) = &mut settings.window else {
            return Poll::Pending;
        };
        window.requests.poll_recv(cx).map(|request| match request {
            Some(request) => WindowEvent::Request(request),
            None => WindowEvent::Closed,
        })
    })
}

impl<P: Platform> Controller<P> {
    /// Saves go to `file`, which was read at start: until this is called,
    /// there is nowhere to save.
    pub fn keep_config_file(&mut self, file: ConfigFile) {
        self.settings.file = file;
    }

    /// Where a settings window's link goes.
    pub fn window_opener(&self) -> WindowOpener {
        self.settings.opener.clone()
    }

    pub(super) fn window_event(&mut self, event: WindowEvent) {
        match event {
            WindowEvent::Opened(link) => {
                self.settings.window = Some(link);
                self.settings.answer = None;
            }
            WindowEvent::Closed => self.settings.window = None,
            WindowEvent::Request(Request { id, ask }) => {
                let outcome = match ask {
                    Ask::Save {
                        settings,
                        replace,
                        without_modifier,
                    } => self.save_settings(&settings, replace, without_modifier),
                };
                self.settings.answer = Some(Answer { id, outcome });
            }
        }
    }

    fn save_settings(&mut self, draft: &Draft, replace: bool, without_modifier: bool) -> Outcome {
        let config = match draft.to_config().and_then(|config| self.allowed(config)) {
            Ok(config) => config,
            Err(refusal) => return Outcome::Refused(refusal),
        };
        if !without_modifier && let Some(action) = self.new_without_modifier(&config.hotkeys) {
            return Outcome::Confirm(Confirm::WithoutModifier(action));
        }
        match self.settings.file.save(&config, replace) {
            Ok(()) => {
                self.use_saved(config);
                Outcome::Saved
            }
            Err(SaveError::Stale(stale)) => Outcome::Confirm(Confirm::Replace(stale)),
            Err(SaveError::NoPath) => Outcome::Failed(
                "there is no config directory: neither XDG_CONFIG_HOME nor HOME is set".to_owned(),
            ),
            Err(SaveError::Write(err)) => {
                Outcome::Failed(format!("the config file could not be written: {err}"))
            }
        }
    }

    /// `config` when the daemon as it runs now may take it; else why not.
    /// Where the desktop delivers the hotkeys, its settings choose the keys,
    /// so `[hotkeys]` stays as it is. Where the program reads the
    /// keyboards, the clicker's key may be no hotkey: neither of the new
    /// bindings, even when the save moves the hotkeys to the desktop (that
    /// applies from the next start), nor of the bindings in use while the
    /// reader cannot take the new ones yet, since the clicker settings apply
    /// from the next start.
    fn allowed(&self, config: Config) -> Result<Config, Refusal> {
        if self.platform.input_reader() == InputReader::Desktop {
            return match changed_action(&self.settings.saved.hotkeys, &config.hotkeys) {
                Some(action) => Err(Refusal::new(
                    Field::Hotkey(action),
                    "the desktop delivers the hotkeys: change them in System Settings, under \
                     Shortcuts"
                        .to_owned(),
                )),
                None => Ok(config),
            };
        }
        // Checked here whatever route the save names: the reader of the
        // keyboards matches the new bindings until the daemon restarts.
        if let Some((action, reason)) = check::on_the_clicker_key(&config.hotkeys, &config.clicker)
        {
            return Err(Refusal::new(Field::Hotkey(action), reason));
        }
        if !self.rebinds_at_once()
            && let Some((action, reason)) =
                check::on_the_clicker_key(&self.config.hotkeys, &config.clicker)
        {
            return Err(Refusal::new(
                Field::Hotkey(action),
                format!("{reason}, or save once the program is idle"),
            ));
        }
        Ok(config)
    }

    /// Whether the reader would take new bindings right after a save: the
    /// controller is idle and opens nothing.
    fn rebinds_at_once(&self) -> bool {
        matches!(self.state, State::Idle) && self.opening.is_none()
    }

    /// The first action whose binding has no modifier and is not saved
    /// already: one the user has not agreed to yet.
    fn new_without_modifier(&self, bindings: &Bindings) -> Option<Action> {
        let saved = &self.settings.saved.hotkeys;
        bindings
            .actions()
            .into_iter()
            .find_map(|(action, binding)| {
                (binding.modifiers() == Modifiers::NONE && saved.binding(action) != binding)
                    .then_some(action)
            })
    }

    /// Takes a saved config into use: the clicker, playback and tray
    /// settings at once (a run that goes on keeps its own), the hotkeys
    /// once the controller is idle. The file is valid now.
    fn use_saved(&mut self, saved: Config) {
        self.config.tray_start_delay = saved.tray_start_delay;
        self.config.clicker = saved.clicker;
        self.config.playback = saved.playback;
        self.config_error = None;
        self.settings.saved = saved;
    }

    /// Starts the reader again with the saved hotkeys, once nothing is
    /// going on: no run, start or recording, and no open, after which the
    /// reader starts again anyway (and which it must know the devices of).
    pub(super) fn rebind_when_idle(&mut self) {
        let saved = self.settings.saved.hotkeys;
        if saved == self.config.hotkeys || !self.rebinds_at_once() {
            return;
        }
        self.config.hotkeys = saved;
        // The reader in use goes first: two never read at once.
        self.hotkeys = HotkeySource::none();
        self.start_reader();
    }

    /// Shows the window what it shows, when one is open.
    pub(super) fn publish_window(&self) {
        let Some(window) = &self.settings.window else {
            return;
        };
        let view = WindowView {
            settings: Draft::from(&self.settings.saved),
            running: Draft::from(&self.config).input,
            activity: self.activity(),
            hotkeys: ShownHotkeys {
                toggle_clicking: self.shown_hotkey(Action::ToggleClicking),
                toggle_recording: self.shown_hotkey(Action::ToggleRecording),
                toggle_playback: self.shown_hotkey(Action::TogglePlayback),
            },
            no_recording: self.no_recording(),
            problems: self.problems(),
            answer: self.settings.answer.clone(),
        };
        window.view.send_if_modified(|shown| {
            let changed = shown.as_ref() != Some(&view);
            if changed {
                *shown = Some(view);
            }
            changed
        });
    }
}

/// The first action whose binding differs between `from` and `to`.
fn changed_action(from: &Bindings, to: &Bindings) -> Option<Action> {
    Action::ALL
        .into_iter()
        .find(|action| from.binding(*action) != to.binding(*action))
}
