//! The config the daemon starts with, on either system.

use auto_clicker_core::config::{Config, ConfigFile, config_path};
use auto_clicker_core::log;
use auto_clicker_core::message::ConfigProblem;

/// The config, its error as the tray shows it, and the file as read. A
/// config that cannot be used is logged, and the defaults are used.
pub fn load_config() -> (Config, Option<ConfigProblem>, ConfigFile) {
    let Some(path) = config_path() else {
        let error = ConfigProblem::NoDirectory;
        log::line(format_args!("auto-clicker: {error}; using the defaults"));
        return (Config::default(), Some(error), ConfigFile::none());
    };
    let loaded = Config::load(&path);
    let error = loaded.error.as_ref().map(ConfigProblem::from);
    if let Some(error) = &error {
        log::line(format_args!("auto-clicker: {error}; using the defaults"));
    }
    (loaded.config, error, loaded.file)
}
