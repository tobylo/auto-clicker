//! Which language the program speaks: Swedish for a Swedish locale,
//! English otherwise. The window and the tray both ask here.

use std::ffi::OsString;

/// A language the program has text in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    Swedish,
}

impl Language {
    /// The language of the environment the program runs in.
    pub fn of_environment() -> Language {
        Language::from_variables(|name| std::env::var_os(name))
    }

    /// The language the locale variables `get` gives: the first of
    /// `LANGUAGE` (a list, the first entry), `LC_ALL`, `LC_MESSAGES` and
    /// `LANG` that is set and not empty decides, as gettext reads them.
    /// `LANGUAGE` counts only with a locale other than "C" set, as gettext
    /// has it.
    pub fn from_variables(get: impl Fn(&str) -> Option<OsString>) -> Language {
        let set = |name: &str| get(name).filter(|value| !value.is_empty());
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().find_map(set);
        let plain = locale.as_ref().is_none_or(|locale| {
            matches!(locale.to_str(), Some("C" | "POSIX"))
                || locale.to_str().is_some_and(|l| l.starts_with("C."))
        });
        let listed = set("LANGUAGE").filter(|_| !plain).and_then(|list| {
            list.to_str()
                .and_then(|l| l.split(':').next())
                .map(OsString::from)
        });
        match listed.or(locale) {
            Some(name) if is_swedish(&name) => Language::Swedish,
            _ => Language::English,
        }
    }

    /// The code the translations are filed under.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Swedish => "sv",
        }
    }
}

/// Whether a locale name ("sv_SE.UTF-8", "sv", "sv_FI@euro") is Swedish.
fn is_swedish(name: &OsString) -> bool {
    name.to_str().is_some_and(|name| {
        name == "sv"
            || name.starts_with("sv_")
            || name.starts_with("sv.")
            || name.starts_with("sv@")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn language(variables: &[(&str, &str)]) -> Language {
        Language::from_variables(|name| {
            variables
                .iter()
                .find(|(set, _)| *set == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn swedish_locales_give_swedish_and_the_rest_english() {
        for (name, expected) in [
            ("sv_SE.UTF-8", Language::Swedish),
            ("sv_FI@euro", Language::Swedish),
            ("sv", Language::Swedish),
            ("sv.UTF-8", Language::Swedish),
            ("sv@euro", Language::Swedish),
            ("svenska", Language::English),
            ("en_US.UTF-8", Language::English),
            ("svx_YZ", Language::English),
            ("de_DE.UTF-8", Language::English),
        ] {
            assert_eq!(language(&[("LANG", name)]), expected, "{name}");
        }
        assert_eq!(language(&[]), Language::English);
    }

    #[test]
    fn each_language_has_the_code_its_translations_are_filed_under() {
        assert_eq!(Language::English.code(), "en");
        assert_eq!(Language::Swedish.code(), "sv");
    }

    #[test]
    fn the_first_variable_set_decides_as_gettext_reads_them() {
        let sv = "sv_SE.UTF-8";
        let en = "en_GB.UTF-8";
        assert_eq!(
            language(&[("LANG", en), ("LC_MESSAGES", sv)]),
            Language::Swedish
        );
        assert_eq!(
            language(&[("LC_MESSAGES", sv), ("LC_ALL", en)]),
            Language::English
        );
        assert_eq!(language(&[("LC_ALL", ""), ("LANG", sv)]), Language::Swedish);
        assert_eq!(
            language(&[("LANG", en), ("LANGUAGE", "sv:en")]),
            Language::Swedish
        );
        // LANGUAGE is not read under the C locale.
        assert_eq!(
            language(&[("LANG", "C.UTF-8"), ("LANGUAGE", "sv")]),
            Language::English
        );
        assert_eq!(language(&[("LANGUAGE", "sv")]), Language::English);
    }
}
