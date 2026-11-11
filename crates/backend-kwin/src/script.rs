//! The two KWin scripts, embedded in the binary, each under one fixed plugin
//! name (brief, "KWin scripts").

use crate::token::Token;

const COMMON: &str = include_str!("scripts/common.js");
const LAYOUT: &str = include_str!("scripts/layout.js");
const POSITION: &str = include_str!("scripts/position.js");

/// Which script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Script {
    /// Loaded while the daemon runs; reports the layout.
    Layout,
    /// Loaded while a recording runs; reports the layout once and the
    /// pointer.
    Position,
}

impl Script {
    pub(crate) const ALL: [Script; 2] = [Script::Layout, Script::Position];

    /// The plugin name KWin knows it by. Every load unloads it first.
    pub(crate) fn plugin(self) -> &'static str {
        match self {
            Script::Layout => "auto-clicker-layout",
            Script::Position => "auto-clicker-position",
        }
    }

    /// The source of one load, with `token` baked in.
    pub(crate) fn source(self, token: &Token) -> String {
        let own = match self {
            Script::Layout => LAYOUT,
            Script::Position => POSITION,
        };
        format!("const TOKEN = \"{}\";\n\n{COMMON}\n{own}", token.as_str())
    }

    /// The file one load is written to: the plugin name and the load's
    /// token, so every load has a name of its own.
    pub(crate) fn file_name(self, token: &Token) -> String {
        format!("{}-{}.js", self.plugin(), token.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receiver::{INTERFACE, PATH};

    #[test]
    fn each_load_has_its_token_and_reaches_the_receiver() {
        let token = Token::new(&mut fastrand::Rng::with_seed(1));
        for script in Script::ALL {
            let source = script.source(&token);
            assert!(source.starts_with(&format!("const TOKEN = \"{}\";", token.as_str())));
            assert!(source.contains(&format!("\"{PATH}\"")));
            assert!(source.contains(&format!("\"{INTERFACE}\"")));
            assert!(source.contains("\"io.github.tobylo.AutoClicker\""));
            let name = script.file_name(&token);
            assert!(name.starts_with(script.plugin()) && name.ends_with(".js"));
        }
        assert!(Script::Position.source(&token).contains("cursorPosChanged"));
        assert!(!Script::Layout.source(&token).contains("cursorPos"));
    }
}
