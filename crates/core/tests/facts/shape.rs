//! The one shape docs/facts.json may have. serde refuses unknown keys, wrong
//! types, unknown values and a second JSON value; `problems` then refuses what
//! types cannot say. The project page's build checks the same shape
//! (tobylo/auto-clicker-site, src/data/validate.ts).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Facts {
    pub version: u32,
    /// Pages of this repository the project page links to, by name.
    pub links: BTreeMap<String, String>,
    pub platforms: Vec<Platform>,
    pub hotkeys: Hotkeys,
    pub figures: Vec<Figure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Available,
    Soon,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Platform {
    pub id: String,
    pub label: String,
    pub system: String,
    pub status: Status,
    pub scope: String,
    /// The README heading whose code blocks hold this platform's commands.
    #[serde(default, deserialize_with = "present")]
    pub readme: Option<String>,
    pub lines: Vec<Line>,
    #[serde(default, deserialize_with = "present")]
    pub note: Option<Note>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Line {
    Command(Command),
    Remark(Remark),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Command {
    pub command: String,
    /// The README has the command and the page leaves it to the install guide.
    #[serde(default, deserialize_with = "present")]
    pub guide_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Remark {
    pub remark: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Note {
    pub text: String,
    pub link: String,
    pub link_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hotkeys {
    pub modifiers: String,
    pub bindings: Vec<Hotkey>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hotkey {
    pub key: String,
    pub name: String,
    pub does: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Basis {
    Measured,
    Rule,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Figure {
    pub id: String,
    pub value: String,
    pub unit: String,
    pub label: String,
    pub basis: Basis,
    pub detail: String,
    pub source: Source,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: String,
    pub quote: String,
}

/// An optional field that, when present, holds a value: `null` is refused.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// The facts in `json`, or every problem with them.
pub fn parse(json: &str) -> Result<Facts, Vec<String>> {
    let facts: Facts = serde_json::from_str(json).map_err(|e| vec![e.to_string()])?;
    let problems = facts.problems();
    if problems.is_empty() {
        Ok(facts)
    } else {
        Err(problems)
    }
}

impl Facts {
    fn problems(&self) -> Vec<String> {
        let mut p = Problems::default();
        p.check(self.version == 1, || "version must be 1".into());
        p.check(!self.links.is_empty(), || "links must not be empty".into());
        for (name, path) in &self.links {
            p.check(is_slug(name), || {
                format!("links: {name:?} is not lowercase words joined by -")
            });
            p.check(is_repo_path(path), || {
                format!("links.{name}: must be a path in this repository")
            });
        }
        p.list("platforms", &self.platforms);
        let headings = self.platforms.iter().filter_map(|x| x.readme.as_deref());
        p.check(distinct(headings), || {
            "platforms: each README heading belongs to one platform".into()
        });
        self.hotkeys.problems(&mut p);
        p.list("figures", &self.figures);
        p.0
    }
}

impl Platform {
    fn platform_problems(&self, at: &str, p: &mut Problems) {
        p.texts(
            at,
            &[
                ("label", &self.label),
                ("system", &self.system),
                ("scope", &self.scope),
            ],
        );
        for (i, line) in self.lines.iter().enumerate() {
            line.problems(&format!("{at} line {}", i + 1), p);
        }
        if let Some(note) = &self.note {
            note.problems(at, p);
        }
        match self.status {
            Status::Available => self.available_problems(at, p),
            Status::Soon => self.soon_problems(at, p),
        }
    }

    fn available_problems(&self, at: &str, p: &mut Problems) {
        p.check(self.readme.as_deref().is_some_and(is_text), || {
            format!("{at}: an available platform names its README heading (readme)")
        });
        let has_command = self.lines.iter().any(|l| matches!(l, Line::Command(_)));
        p.check(has_command, || {
            format!("{at}: an available platform has commands")
        });
    }

    fn soon_problems(&self, at: &str, p: &mut Problems) {
        p.check(self.readme.is_none(), || {
            format!("{at}: a platform that is soon has no README heading")
        });
        p.check(self.lines.is_empty(), || {
            format!("{at}: a platform that is soon has no install lines")
        });
    }
}

impl Line {
    fn problems(&self, at: &str, p: &mut Problems) {
        match self {
            Line::Command(c) => {
                p.texts(at, &[("command", &c.command)]);
                p.check(c.guide_only != Some(false), || {
                    format!("{at}: guideOnly may only be true")
                });
            }
            Line::Remark(r) => p.texts(at, &[("remark", &r.remark)]),
        }
    }

    /// The command on this line, if it is one.
    pub fn command(&self) -> Option<&str> {
        match self {
            Line::Command(c) => Some(&c.command),
            Line::Remark(_) => None,
        }
    }
}

impl Note {
    fn problems(&self, at: &str, p: &mut Problems) {
        p.texts(
            at,
            &[
                ("note text", &self.text),
                ("note linkText", &self.link_text),
            ],
        );
        p.check(is_repo_path(&self.link), || {
            format!("{at}: note link must be a path in this repository")
        });
    }
}

impl Hotkeys {
    fn problems(&self, p: &mut Problems) {
        p.texts("hotkeys", &[("modifiers", &self.modifiers)]);
        p.check(!self.bindings.is_empty(), || {
            "hotkeys: bindings must not be empty".into()
        });
        for (i, hotkey) in self.bindings.iter().enumerate() {
            let at = format!("hotkey {}", i + 1);
            p.texts(
                &at,
                &[
                    ("key", &hotkey.key),
                    ("name", &hotkey.name),
                    ("does", &hotkey.does),
                ],
            );
        }
        let keys = self.bindings.iter().map(|h| h.key.as_str());
        p.check(distinct(keys), || "hotkeys: each key appears once".into());
    }
}

impl Figure {
    fn figure_problems(&self, at: &str, p: &mut Problems) {
        p.texts(
            at,
            &[
                ("value", &self.value),
                ("label", &self.label),
                ("detail", &self.detail),
            ],
        );
        p.check(self.unit.is_empty() || is_text(&self.unit), || {
            format!("{at}: unit must be text, or empty")
        });
        p.check(is_repo_path(&self.source.path), || {
            format!("{at}: source path must be a path in this repository")
        });
        p.texts(at, &[("source quote", &self.source.quote)]);
    }
}

#[derive(Default)]
struct Problems(Vec<String>);

impl Problems {
    fn check(&mut self, ok: bool, problem: impl FnOnce() -> String) {
        if !ok {
            self.0.push(problem());
        }
    }

    /// Each named field is one line of text.
    fn texts(&mut self, at: &str, fields: &[(&str, &str)]) {
        for (name, value) in fields {
            self.check(is_text(value), || {
                format!("{at}: {name} must be one line of text")
            });
        }
    }

    /// `items` is a non-empty list of distinct ids, each a slug, and each item
    /// with its own problems.
    fn list<T: Listed>(&mut self, name: &str, items: &[T]) {
        self.check(!items.is_empty(), || format!("{name} must not be empty"));
        for (i, item) in items.iter().enumerate() {
            let at = format!("{name} {}", i + 1);
            self.check(is_slug(item.id()), || {
                format!("{at}: id must be lowercase words joined by -")
            });
            item.problems(&at, self);
        }
        self.check(distinct(items.iter().map(Listed::id)), || {
            format!("{name}: each id appears once")
        });
    }
}

/// An item of a list whose ids are distinct.
trait Listed {
    fn id(&self) -> &str;
    fn problems(&self, at: &str, p: &mut Problems);
}

impl Listed for Platform {
    fn id(&self) -> &str {
        &self.id
    }

    fn problems(&self, at: &str, p: &mut Problems) {
        self.platform_problems(at, p);
    }
}

impl Listed for Figure {
    fn id(&self) -> &str {
        &self.id
    }

    fn problems(&self, at: &str, p: &mut Problems) {
        self.figure_problems(at, p);
    }
}

/// Non-empty, on one line, with no control characters.
fn is_text(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c <= '\x1f' || c == '\x7f')
}

fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// A plain relative path: names of letters, digits, `_`, `.` and `-` joined
/// by `/`, none of them `.` or `..`.
fn is_repo_path(s: &str) -> bool {
    s.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    })
}

fn distinct<'a>(items: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = HashSet::new();
    items.into_iter().all(|item| seen.insert(item))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::parse;

    /// A valid file that stays as it is, whatever docs/facts.json says: an
    /// available platform (whose fifth line is left to the guide) and one
    /// that is soon.
    const VALID: &str = include_str!("valid.json");

    /// The problems with docs/facts.json after `change`, as one text.
    fn problems_after(change: impl FnOnce(&mut Value)) -> String {
        let mut facts: Value = serde_json::from_str(VALID).unwrap();
        change(&mut facts);
        match parse(&facts.to_string()) {
            Ok(_) => String::new(),
            Err(problems) => problems.join("\n"),
        }
    }

    #[track_caller]
    fn refuses(change: impl FnOnce(&mut Value), problem: &str) {
        let problems = problems_after(change);
        assert!(
            problems.contains(problem),
            "expected {problem:?}, got {problems:?}"
        );
    }

    #[test]
    fn accepts_a_valid_file() {
        assert_eq!(problems_after(|_| {}), "");
    }

    #[test]
    fn refuses_unknown_keys_anywhere() {
        refuses(|f| f["extra"] = json!(1), "unknown field `extra`");
        refuses(
            |f| f["figures"][0]["measured"] = json!("x"),
            "unknown field `measured`",
        );
        refuses(
            |f| f["platforms"][0]["note"]["href"] = json!("x"),
            "unknown field `href`",
        );
    }

    #[test]
    fn refuses_wrong_types_and_values() {
        refuses(|f| f["version"] = json!(2), "version must be 1");
        refuses(
            |f| f["platforms"][1]["status"] = json!("beta"),
            "unknown variant `beta`",
        );
        refuses(
            |f| f["figures"][0]["basis"] = json!("guess"),
            "unknown variant `guess`",
        );
        refuses(
            |f| f["figures"][0]["value"] = json!(10),
            "expected a string",
        );
        refuses(
            |f| f["platforms"][0]["readme"] = json!(null),
            "expected a string",
        );
    }

    #[test]
    fn refuses_a_line_that_is_neither_one_kind_nor_the_other() {
        refuses(
            |f| f["platforms"][0]["lines"][0]["remark"] = json!("x"),
            "untagged enum Line",
        );
        refuses(
            |f| f["platforms"][0]["lines"][0] = json!({}),
            "untagged enum Line",
        );
    }

    #[test]
    fn refuses_text_that_is_empty_or_not_one_line() {
        refuses(
            |f| f["platforms"][0]["lines"][0]["command"] = json!("a\nb"),
            "command must be one line",
        );
        refuses(
            |f| f["hotkeys"]["bindings"][0]["does"] = json!(""),
            "does must be one line",
        );
        refuses(
            |f| f["figures"][0]["unit"] = json!("m\ts"),
            "unit must be text, or empty",
        );
    }

    #[test]
    fn holds_each_status_to_its_rules() {
        refuses(
            |f| f["platforms"][1]["lines"] = json!([{"remark": "x"}]),
            "soon has no install lines",
        );
        refuses(
            |f| f["platforms"][1]["readme"] = json!("Windows"),
            "soon has no README heading",
        );
        refuses(
            |f| {
                f["platforms"][0].as_object_mut().unwrap().remove("readme");
            },
            "names its README heading",
        );
        refuses(
            |f| f["platforms"][0]["lines"] = json!([{"remark": "x"}]),
            "an available platform has commands",
        );
        refuses(
            |f| f["platforms"][0]["lines"][4]["guideOnly"] = json!(false),
            "guideOnly may only be true",
        );
    }

    #[test]
    fn refuses_paths_out_of_the_repository() {
        for path in [
            "../README.md",
            "/etc/passwd",
            "docs//guide",
            "docs/./x.md",
            "",
            "a b.md",
        ] {
            refuses(
                |f| f["links"]["guide"] = json!(path),
                "links.guide: must be a path",
            );
        }
        refuses(
            |f| f["figures"][0]["source"]["path"] = json!("../x"),
            "source path must be a path",
        );
        refuses(
            |f| f["platforms"][0]["note"]["link"] = json!("../x"),
            "note link must be a path",
        );
    }

    #[test]
    fn refuses_names_said_twice_or_not_at_all() {
        refuses(
            |f| f["platforms"][1]["id"] = f["platforms"][0]["id"].clone(),
            "platforms: each id appears once",
        );
        refuses(
            |f| f["figures"][1]["id"] = json!("fastest-interval"),
            "figures: each id appears once",
        );
        refuses(
            |f| f["hotkeys"]["bindings"][1]["key"] = json!("F6"),
            "each key appears once",
        );
        refuses(
            |f| f["hotkeys"]["bindings"] = json!([]),
            "bindings must not be empty",
        );
        refuses(|f| f["links"] = json!({}), "links must not be empty");
        refuses(
            |f| f["platforms"] = json!([]),
            "platforms must not be empty",
        );
        refuses(|f| f["figures"] = json!([]), "figures must not be empty");
        refuses(
            |f| f["platforms"][0]["id"] = json!("Arch"),
            "platforms 1: id must be lowercase",
        );
        refuses(
            |f| f["figures"][0]["id"] = json!("fast-"),
            "figures 1: id must be lowercase",
        );
        refuses(
            |f| f["links"]["Guide"] = json!("README.md"),
            "is not lowercase words",
        );
        refuses(
            |f| {
                f["platforms"][1] = f["platforms"][0].clone();
                f["platforms"][1]["id"] = json!("other");
            },
            "each README heading belongs to one platform",
        );
    }

    #[test]
    fn refuses_more_than_one_value() {
        let twice = format!("{VALID}{VALID}");
        let problems = parse(&twice).err().unwrap().join("\n");
        assert!(problems.contains("trailing characters"), "{problems}");
    }

    #[test]
    fn lists_every_problem_at_once() {
        let problems = problems_after(|f| {
            f["version"] = json!(2);
            f["figures"][0]["label"] = json!("");
        });
        assert!(problems.contains("version must be 1\n"), "{problems}");
        assert!(problems.contains("label must be one line"), "{problems}");
    }
}
