//! The KWin script, embedded in the binary, and the plugin names KWin knows
//! this program's scripts by (brief, "KWin scripts").

use crate::token::Token;

const COMMON: &str = include_str!("scripts/common.js");
const POSITION: &str = include_str!("scripts/position.js");

/// A script under one fixed plugin name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Script {
    /// Loaded while a recording runs; reports the layout at load and on
    /// every change, and the pointer.
    Position,
    /// The layout script versions before W01b loaded for the daemon's life.
    /// Never loaded: the start unloads one a crashed daemon of such a
    /// version left, and removes its files. Can go once no installed
    /// version predates W01b.
    OldLayout,
}

impl Script {
    /// Every name this program's scripts were loaded under: the start's
    /// sweep unloads them all, and removes their files.
    pub(crate) const ALL: [Script; 2] = [Script::OldLayout, Script::Position];

    /// The plugin name KWin knows it by. Every load unloads it first.
    pub(crate) fn plugin(self) -> &'static str {
        match self {
            Script::Position => "auto-clicker-position",
            Script::OldLayout => "auto-clicker-layout",
        }
    }

    /// The file one load is written to: the plugin name and the load's
    /// token, so every load has a name of its own.
    pub(crate) fn file_name(self, token: &Token) -> String {
        format!("{}-{}.js", self.plugin(), token.as_str())
    }
}

/// The position script's source for one load, with `token` baked in.
pub(crate) fn position_source(token: &Token) -> String {
    format!(
        "const TOKEN = \"{}\";\n\n{COMMON}\n{POSITION}",
        token.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receiver::{INTERFACE, PATH};

    #[test]
    fn each_load_has_its_token_and_reaches_the_receiver() {
        let token = Token::new(&mut fastrand::Rng::with_seed(1));
        let source = position_source(&token);
        assert!(source.starts_with(&format!("const TOKEN = \"{}\";", token.as_str())));
        assert!(source.contains(&format!("\"{PATH}\"")));
        assert!(source.contains(&format!("\"{INTERFACE}\"")));
        assert!(source.contains("\"io.github.tobylo.AutoClicker\""));
        for script in Script::ALL {
            let name = script.file_name(&token);
            assert!(name.starts_with(script.plugin()) && name.ends_with(".js"));
        }
    }

    #[test]
    fn the_position_script_reports_each_change_of_the_screens_before_the_pointer() {
        let source = position_source(&Token::new(&mut fastrand::Rng::with_seed(1)));
        for signal in [
            "screensChanged",
            "virtualScreenGeometryChanged",
            "geometryChanged",
            "cursorPosChanged",
        ] {
            assert!(source.contains(signal), "{signal}");
        }
        // Connected first, so on a change the layout goes out before a
        // position on the new screens.
        let layout = source.find("screensChanged.connect").unwrap();
        let pointer = source.find("cursorPosChanged.connect").unwrap();
        assert!(layout < pointer);
    }
}
