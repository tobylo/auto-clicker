//! Which language the program speaks: Swedish for a Swedish locale,
//! English otherwise. The window and the tray both ask here: on Linux from
//! the locale variables, on Windows from the user's display languages,
//! which the Windows backend reads.

use std::ffi::OsString;

/// A language the program has text in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    Swedish,
}

impl Language {
    /// The language of the environment the program runs in. Linux alone:
    /// Windows keeps its language elsewhere ([`Language::from_tags`]).
    #[cfg(unix)]
    pub fn of_environment() -> Language {
        Language::from_variables(|name| std::env::var_os(name))
    }

    /// The language the locale variables `get` gives, as gettext reads
    /// them. With a locale other than "C" set, a `LANGUAGE` list decides:
    /// its first entry the program has text in (English, or Swedish), or
    /// English when it has none. Otherwise the first of `LC_ALL`,
    /// `LC_MESSAGES` and `LANG` that is set and not empty decides.
    pub fn from_variables(get: impl Fn(&str) -> Option<OsString>) -> Language {
        let set = |name: &str| get(name).filter(|value| !value.is_empty());
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().find_map(set);
        let plain = locale.as_ref().is_none_or(|locale| {
            matches!(locale.to_str(), Some("C" | "POSIX"))
                || locale.to_str().is_some_and(|l| l.starts_with("C."))
        });
        // A list with nothing the program has text in means untranslated
        // text, as gettext has it: the locale does not decide then.
        if let Some(list) = set("LANGUAGE").filter(|_| !plain) {
            return list
                .to_str()
                .and_then(first_known)
                .unwrap_or(Language::English);
        }
        match locale {
            Some(name) if is_swedish(&name) => Language::Swedish,
            _ => Language::English,
        }
    }

    /// The language of the first of `tags` the program has text in, or
    /// English when none is. The tags are language names as Windows lists
    /// the user's display languages, most preferred first ("sv-SE",
    /// "en-US"); case does not matter.
    pub fn from_tags<'a>(tags: impl IntoIterator<Item = &'a str>) -> Language {
        tags.into_iter()
            .find_map(language_of_tag)
            .unwrap_or(Language::English)
    }

    /// The code the translations are filed under.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Swedish => "sv",
        }
    }
}

/// The first language in a `LANGUAGE` list ("de:sv:en") the program has
/// text in; "C" is untranslated English.
fn first_known(list: &str) -> Option<Language> {
    list.split(':').find_map(|name| {
        // "C" ends the search in untranslated text, as in gettext.
        if matches!(name, "C" | "POSIX") {
            return Some(Language::English);
        }
        let name = OsString::from(name);
        if is_swedish(&name) {
            Some(Language::Swedish)
        } else if is_english(&name) {
            Some(Language::English)
        } else {
            None
        }
    })
}

/// The language a language name ("sv-SE", "en", "sv-Latn-FI") is in,
/// when the program has text in it: its first part decides.
fn language_of_tag(tag: &str) -> Option<Language> {
    let language = tag.split('-').next()?;
    if language.eq_ignore_ascii_case("sv") {
        Some(Language::Swedish)
    } else if language.eq_ignore_ascii_case("en") {
        Some(Language::English)
    } else {
        None
    }
}

/// Whether a locale name ("en", "en_GB.UTF-8") is English.
fn is_english(name: &OsString) -> bool {
    name.to_str().is_some_and(|name| {
        name == "en"
            || ["en_", "en.", "en@"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
    })
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
    fn the_first_windows_language_with_text_decides() {
        for (tags, expected) in [
            (&["sv-SE"][..], Language::Swedish),
            (&["sv-FI"], Language::Swedish),
            (&["sv"], Language::Swedish),
            (&["SV-se"], Language::Swedish),
            (&["sv-Latn-SE"], Language::Swedish),
            (&["en-US"], Language::English),
            (&["en-GB", "sv-SE"], Language::English),
            (&["de-DE", "sv-SE", "en-US"], Language::Swedish),
            (&["de-DE", "en-US", "sv-SE"], Language::English),
            (&["de-DE"], Language::English),
            (&["svx-SE"], Language::English),
            (&["se-SE"], Language::English),
            (&["sv_SE"], Language::English),
            (&[""], Language::English),
            (&[], Language::English),
        ] {
            assert_eq!(
                Language::from_tags(tags.iter().copied()),
                expected,
                "{tags:?}"
            );
        }
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
        // The first entry the program has text in decides.
        let de_sv = [("LANG", en), ("LANGUAGE", "de:sv")];
        assert_eq!(language(&de_sv), Language::Swedish);
        let de_en = [("LANG", sv), ("LANGUAGE", "de:en_GB:sv")];
        assert_eq!(language(&de_en), Language::English);
        let c_first = [("LANG", sv), ("LANGUAGE", "de:C:sv")];
        assert_eq!(language(&c_first), Language::English);
        let neither = [("LANG", sv), ("LANGUAGE", "de:fr")];
        assert_eq!(language(&neither), Language::English);
        // LANGUAGE is not read under the C locale.
        assert_eq!(
            language(&[("LANG", "C.UTF-8"), ("LANGUAGE", "sv")]),
            Language::English
        );
        assert_eq!(language(&[("LANGUAGE", "sv")]), Language::English);
    }
}
