//! The settings window as the controller sees it: a link that brings the
//! window's requests and takes the view it shows, and the config file the
//! window's saves go to.
//!
//! The controller checks and writes a save, and takes it into use at once:
//! the clicker, playback and tray settings for the next start, the hotkeys
//! by starting the reader again with them. `[input]` picks the platform, so
//! it applies from the daemon's next start.
//!
//! It saves only while idle, with no start pending and no devices opening:
//! a save writes and flushes a file on the controller's thread, which must
//! never keep it from stopping a run, and nothing that is about to start
//! sees its settings change under it.

use std::future::{Future, poll_fn};
use std::task::Poll;

use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use super::capture::Pending;
use super::{Controller, InputReader, Platform, State};
use crate::config::{Config, ConfigFile, Draft, Field, Reason, Refusal, SaveError, Saved, check};
use crate::hotkeys::{Action, Bindings, Capture, Modifiers};
use crate::message::{Failure, Quoted};
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
    pub(super) answer: Option<Answer>,
    /// What the window waits on: a capture, or the desktop's settings.
    pub(super) pending: Option<Pending>,
    /// The number the next request to the reader gets.
    operations: u64,
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
            pending: None,
            operations: 0,
        }
    }

    /// A number no earlier request to the reader had.
    pub(super) fn next_operation(&mut self) -> u64 {
        self.operations = self.operations.wrapping_add(1);
        self.operations
    }

    /// What the reader captures now, if anything.
    pub(super) fn capturing(&self) -> Option<Capture> {
        self.pending.as_ref().and_then(Pending::capture)
    }

    /// When the pending request ends by itself, if one is pending.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(Pending::deadline)
    }
}

/// The next thing a window did; never resolves while none is open and none
/// is opening.
pub(super) fn next_event(settings: &mut Settings) -> impl Future<Output = WindowEvent> + '_ {
    poll_fn(move |cx| {
        // What the window open asked comes first: a window that sent a save
        // and closed has it handled before a new window takes its place.
        if let Some(window) = &mut settings.window
            && let Poll::Ready(request) = window.requests.poll_recv(cx)
        {
            return Poll::Ready(match request {
                Some(request) => WindowEvent::Request(request),
                None => WindowEvent::Closed,
            });
        }
        match settings.openings.poll_recv(cx) {
            Poll::Ready(Some(link)) => Poll::Ready(WindowEvent::Opened(link)),
            // The controller holds an opener itself, so this never ends.
            Poll::Ready(None) | Poll::Pending => Poll::Pending,
        }
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
                self.end_pending();
                self.settings.window = Some(link);
                self.settings.answer = None;
            }
            WindowEvent::Closed => {
                self.end_pending();
                self.settings.window = None;
            }
            WindowEvent::Request(Request { id, ask }) => {
                let outcome = match ask {
                    Ask::Save {
                        settings,
                        replace,
                        without_modifier,
                    } => Some(self.save_settings(&settings, replace, &without_modifier)),
                    Ask::Capture(capture) => self.start_capture(id, capture),
                    Ask::ConfigureShortcuts { activation_token } => {
                        self.configure_shortcuts(id, activation_token)
                    }
                };
                if let Some(outcome) = outcome {
                    let outcome = outcome.bounded();
                    self.settings.answer = Some(Answer { id, outcome });
                }
            }
        }
    }

    fn save_settings(
        &mut self,
        draft: &Draft,
        replace: bool,
        without_modifier: &[Action],
    ) -> Outcome {
        if !self.settled() {
            return Outcome::Busy;
        }
        let config = match draft.to_config().and_then(|config| self.allowed(config)) {
            Ok(config) => config,
            Err(refusal) => return Outcome::Refused(refusal),
        };
        let unagreed = self.new_without_modifier(&config.hotkeys, without_modifier);
        if !unagreed.is_empty() {
            return Outcome::Confirm(Confirm::WithoutModifier(unagreed));
        }
        let outcome = match self.settings.file.save(&config, replace) {
            Ok(Saved::Synced) => Outcome::Saved,
            Ok(Saved::Unsynced(err)) => Outcome::SavedUnsynced(Quoted::new(err.to_string())),
            Err(SaveError::Stale(stale)) => return Outcome::Confirm(Confirm::Replace(stale)),
            Err(SaveError::NoPath) => return Outcome::Failed(Failure::NoConfigDirectory),
            Err(SaveError::Write(err)) => {
                return Outcome::Failed(Failure::Write(Quoted::new(err.to_string())));
            }
        };
        self.use_saved(config);
        outcome
    }

    /// Whether nothing goes on: no run, start or recording, no open, and
    /// nothing the window waits on.
    pub(super) fn settled(&self) -> bool {
        matches!(self.state, State::Idle)
            && self.opening.is_none()
            && self.settings.pending.is_none()
    }

    /// `config` when the daemon as it runs now may take it; else why not.
    /// Where the desktop delivers the hotkeys, its settings choose the keys,
    /// so `[hotkeys]` stays as it is. Where the program reads the
    /// keyboards, no hotkey may be on the clicker's key, even when the save
    /// moves the hotkeys to the desktop: that applies from the next start,
    /// and the reader matches the new bindings until then.
    fn allowed(&self, config: Config) -> Result<Config, Refusal> {
        if self.platform.input_reader() == InputReader::Desktop {
            return match changed_action(&self.settings.saved.hotkeys, &config.hotkeys) {
                Some(action) => Err(Refusal::new(Field::Hotkey(action), Reason::DesktopChooses)),
                None => Ok(config),
            };
        }
        match check::on_the_clicker_key(&config.hotkeys, &config.clicker) {
            Some((action, reason)) => Err(Refusal::new(Field::Hotkey(action), reason)),
            None => Ok(config),
        }
    }

    /// Every action whose binding has no modifier, is not saved already and
    /// is not among those the user `agreed` to.
    fn new_without_modifier(&self, bindings: &Bindings, agreed: &[Action]) -> Vec<Action> {
        let saved = &self.settings.saved.hotkeys;
        bindings
            .actions()
            .into_iter()
            .filter(|&(action, binding)| {
                binding.modifiers() == Modifiers::NONE
                    && saved.binding(action) != binding
                    && !agreed.contains(&action)
            })
            .map(|(action, _)| action)
            .collect()
    }

    /// Takes a saved config into use: the clicker, playback and tray
    /// settings for the next start, the hotkeys at once. The controller is
    /// settled, so the reader can start again: it knows every device. The
    /// file is valid now.
    fn use_saved(&mut self, saved: Config) {
        self.config.tray_start_delay = saved.tray_start_delay;
        self.config.clicker = saved.clicker;
        self.config.playback = saved.playback;
        self.config_error = None;
        if saved.hotkeys != self.config.hotkeys {
            self.config.hotkeys = saved.hotkeys;
            self.restart_reader();
        }
        self.settings.saved = saved;
    }

    /// Shows the window what it shows, when one is open.
    pub(super) fn publish_window(&self) {
        let Some(window) = &self.settings.window else {
            return;
        };
        let view = WindowView {
            settings: Draft::from(&self.settings.saved),
            running: Draft::from(&self.config).input,
            routes: super::routes(&self.platform),
            activity: self.activity(),
            hotkeys: ShownHotkeys {
                toggle_clicking: self.shown_hotkey(Action::ToggleClicking),
                toggle_recording: self.shown_hotkey(Action::ToggleRecording),
                toggle_playback: self.shown_hotkey(Action::TogglePlayback),
            },
            no_recording: self.no_recording(),
            problems: self.problems(),
            answer: self.settings.answer.clone(),
        }
        .bounded();
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
