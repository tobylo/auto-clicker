//! The config file: `$XDG_CONFIG_HOME/auto-clicker/config.toml`.
//!
//! The file is parsed once into a validated [`Config`]. A missing file means
//! the built-in defaults, and nothing is written. A file that cannot be read
//! or does not validate is left as it is: the program runs on the defaults
//! and reports the error, with the file, line and reason.
//!
//! ```toml
//! version = 1
//! tray_start_delay = 5          # seconds before a start from the tray clicks
//!
//! [clicker]
//! mode = "click"                # click, double_click, hold_button, key_repeat, hold_key
//! interval_ms = 100             # or clicks_per_second = 10, not both
//! jitter_percent = 0            # 0 to 50
//! button = "left"               # left, right, middle, back, forward
//! key = "space"                 # a key name, or a Linux key code
//! position = "cursor"           # or { x = 32768, y = 32768 } on 0..65535
//! repeat = "until_stopped"      # or a number of actions
//!
//! [hotkeys]                     # modifiers Ctrl, Shift, Alt, Meta; any case
//! toggle_clicking = "Ctrl+Shift+F6"
//! toggle_recording = "Ctrl+Shift+F7"
//! toggle_playback = "Ctrl+Shift+F8"
//!
//! [input]
//! output = "auto"               # auto, kernel (/dev/uinput) or portal (the
//!                               # desktop's remote input portal, which asks once)
//! hotkeys = "auto"              # auto, kernel (reads the keyboards) or portal
//!                               # (the desktop's shortcuts, which asks once)
//!                               # auto: kernel where the program has access
//!
//! [playback]
//! speed = 1.0                   # 0.25 to 4 times the recorded speed
//! repeat = 1                    # a number of plays, or "until_stopped"
//! ```

pub(crate) mod check;
mod draft;
mod error;
mod file;
mod hotkeys;
mod names;
mod raw;
mod read;
mod reason;
mod validate;
mod write;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use draft::{
    ClickerDraft, CountDraft, Draft, Field, HotkeysDraft, InputDraft, IntervalDraft, PlaybackDraft,
    PositionDraft, Refusal, spell_interval,
};
pub use error::{ConfigError, Problem};
use file::Seen;
pub use file::{ConfigFile, SaveError, Saved, Stale};
pub use names::{ButtonName, ModeName, RouteName};
pub use reason::Reason;
pub use write::render;

use crate::clicker::ClickerSettings;
use crate::hotkeys::Bindings;
use crate::playback::PlaybackSettings;

/// The longest countdown before a start from the tray: an hour.
pub const MAX_TRAY_START_DELAY: Duration = Duration::from_secs(60 * 60);

/// Everything the config file sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// How long "Start clicking" in the tray menu counts down before it
    /// clicks, so the user can switch to the game. The hotkey starts at once.
    pub tray_start_delay: Duration,
    pub clicker: ClickerSettings,
    pub hotkeys: Bindings,
    pub input: InputSettings,
    pub playback: PlaybackSettings,
}

/// `[input]`: how the program reaches the input devices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputSettings {
    /// Where clicks and keys go.
    pub output: OutputRoute,
    /// Where hotkeys come from.
    pub hotkeys: HotkeyRoute,
}

/// The route hotkeys take to the program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HotkeyRoute {
    /// The keyboards where the program may read one at start, else the
    /// desktop's shortcuts where the desktop offers them.
    #[default]
    Auto,
    /// The keyboards, read from `/dev/input`, which needs the `input` group.
    Kernel,
    /// The desktop's GlobalShortcuts portal. The desktop asks the user once,
    /// and its settings choose the keys from then on.
    Portal,
}

impl HotkeyRoute {
    /// Whether the program may end up matching the hotkeys itself, from the
    /// keyboards: on the kernel route, and on the automatic one, which may
    /// pick it.
    pub fn may_read_keyboards(self) -> bool {
        self != HotkeyRoute::Portal
    }
}

/// The route clicks and keys take to the desktop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputRoute {
    /// `/dev/uinput` where the program may open it at start, else the
    /// desktop's remote input portal where the desktop offers it.
    #[default]
    Auto,
    /// Virtual devices through `/dev/uinput`.
    Kernel,
    /// The desktop's RemoteDesktop portal and libei. The desktop asks the
    /// user once.
    Portal,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            tray_start_delay: Duration::from_secs(5),
            clicker: ClickerSettings::default(),
            hotkeys: Bindings::default(),
            input: InputSettings::default(),
            playback: PlaybackSettings::default(),
        }
    }
}

/// The result of loading the config: always a config to run on, the error,
/// if the file could not be used, and the file as it was read, for saving.
#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    pub error: Option<ConfigError>,
    pub file: ConfigFile,
}

impl Config {
    /// Parses a config file's text.
    pub fn parse(text: &str) -> Result<Config, Problem> {
        toml::from_str::<raw::RawConfig>(text)
            .map_err(|err| Problem::invalid(text, err.span(), err.message().to_owned()))
            .and_then(|raw| {
                validate::validate(raw).map_err(|bad| Problem::invalid(text, bad.span, bad.reason))
            })
    }

    /// Loads the file at `path`. Never writes to it.
    pub fn load(path: &Path) -> Loaded {
        let parsed = read::read_config(path).and_then(|bytes| match bytes {
            None => Ok((Config::default(), Seen::Missing)),
            Some(bytes) => parse_bytes(&bytes).map(|config| (config, Seen::Text(bytes))),
        });
        match parsed {
            Ok((config, seen)) => Loaded {
                config,
                error: None,
                file: ConfigFile::new(path, seen),
            },
            Err(problem) => Loaded {
                config: Config::default(),
                error: Some(ConfigError::new(path, problem)),
                file: ConfigFile::new(path, Seen::Unusable),
            },
        }
    }
}

fn parse_bytes(bytes: &[u8]) -> Result<Config, Problem> {
    let text = std::str::from_utf8(bytes).map_err(|_| Problem::Invalid {
        line: None,
        reason: "the file is not UTF-8 text".to_owned(),
    })?;
    Config::parse(text)
}

/// Where the config file is: `$XDG_CONFIG_HOME/auto-clicker/config.toml`, or
/// `~/.config/auto-clicker/config.toml` when `XDG_CONFIG_HOME` is unset or
/// not absolute. `None` when neither gives an absolute directory.
pub fn config_path() -> Option<PathBuf> {
    config_path_from(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

fn config_path_from(xdg_config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let absolute = |dir: OsString| Some(PathBuf::from(dir)).filter(|dir| dir.is_absolute());
    xdg_config_home
        .and_then(absolute)
        .or_else(|| home.and_then(absolute).map(|home| home.join(".config")))
        .map(|dir| dir.join("auto-clicker").join("config.toml"))
}

#[cfg(test)]
mod file_tests;
#[cfg(test)]
mod hotkey_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod playback_tests;
#[cfg(test)]
mod tests;
