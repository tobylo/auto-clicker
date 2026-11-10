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
//! toggle_clicking = "Ctrl+F6"
//! toggle_recording = "Ctrl+F7"
//! toggle_playback = "Ctrl+F8"
//!
//! [input]
//! output = "kernel"             # kernel (/dev/uinput) or portal (the desktop's
//!                               # remote input portal, which asks once)
//! ```

mod error;
mod hotkeys;
mod raw;
mod read;
mod validate;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use error::{ConfigError, Problem};

use crate::clicker::ClickerSettings;
use crate::hotkeys::Bindings;

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
}

/// `[input]`: how the program reaches the input devices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputSettings {
    /// Where clicks and keys go.
    pub output: OutputRoute,
}

/// The route clicks and keys take to the desktop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputRoute {
    /// Virtual devices through `/dev/uinput`.
    #[default]
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
        }
    }
}

/// The result of loading the config: always a config to run on, and the
/// error, if the file could not be used.
#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    pub error: Option<ConfigError>,
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
        let parsed = read::read_config(path).and_then(|bytes| bytes.map(parse_bytes).transpose());
        match parsed {
            Ok(config) => Loaded {
                config: config.unwrap_or_default(),
                error: None,
            },
            Err(problem) => Loaded {
                config: Config::default(),
                error: Some(ConfigError::new(path, problem)),
            },
        }
    }
}

fn parse_bytes(bytes: Vec<u8>) -> Result<Config, Problem> {
    let text = String::from_utf8(bytes).map_err(|_| Problem::Invalid {
        line: None,
        reason: "the file is not UTF-8 text".to_owned(),
    })?;
    Config::parse(&text)
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
mod hotkey_tests;
#[cfg(test)]
mod tests;
