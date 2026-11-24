//! docs/facts.json, the facts the project page shows (tobylo/auto-clicker-site
//! reads the file at a commit it pins), against this repository: the file has
//! its one shape, each available platform's commands are the code blocks
//! under its README heading, the hotkeys are the README's table and the
//! program's default bindings, the fastest stable interval is the interval
//! floor, and every file it names exists and holds the quote it cites.

// Test code: a panic is the failure report.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod readme;
mod shape;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use auto_clicker_core::clicker::INTERVAL_FLOOR;
use auto_clicker_core::hotkeys::{Action, Binding, Bindings};

use shape::{Basis, Facts, Status};

const FACTS: &str = include_str!("../../../../docs/facts.json");
const README: &str = include_str!("../../../../README.md");

fn facts() -> Facts {
    shape::parse(FACTS)
        .unwrap_or_else(|problems| panic!("docs/facts.json:\n{}", problems.join("\n")))
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Whether `path`, relative to `root`, is a regular file with no symbolic
/// link along it, so it is the file GitHub shows at that path.
fn is_file_in(root: &Path, path: &str) -> bool {
    let mut at = root.to_path_buf();
    let no_links = path.split('/').all(|part| {
        at.push(part);
        at.symlink_metadata()
            .is_ok_and(|m| !m.file_type().is_symlink())
    });
    no_links && at.is_file()
}

#[test]
fn the_file_has_its_one_shape() {
    facts();
}

#[test]
fn each_available_platforms_commands_are_its_readme_sections() {
    for platform in facts().platforms {
        if platform.status != Status::Available {
            continue;
        }
        let heading = platform
            .readme
            .expect("an available platform names its heading");
        let from_readme =
            readme::section_commands(README, &heading).unwrap_or_else(|e| panic!("{e}"));
        let from_file: Vec<&str> = platform
            .lines
            .iter()
            .filter_map(|line| line.command())
            .collect();
        assert_eq!(
            from_readme, from_file,
            "README.md's commands under \"{heading}\" (left) differ from docs/facts.json's for {} (right)",
            platform.id
        );
    }
}

#[test]
fn the_hotkeys_are_the_readmes_table() {
    let hotkeys = facts().hotkeys;
    let from_readme = readme::hotkey_column(README).unwrap_or_else(|e| panic!("{e}"));
    let from_file: Vec<String> = hotkeys
        .bindings
        .iter()
        .map(|hotkey| format!("{}+{}", hotkeys.modifiers, hotkey.key))
        .collect();
    assert_eq!(
        from_readme, from_file,
        "README.md's hotkey table (left) differs from docs/facts.json's hotkeys (right)"
    );
}

#[test]
fn the_hotkeys_are_the_default_bindings_in_action_order() {
    let hotkeys = facts().hotkeys;
    assert_eq!(hotkeys.bindings.len(), Action::ALL.len());
    let defaults = Bindings::default();
    for (action, hotkey) in Action::ALL.into_iter().zip(&hotkeys.bindings) {
        let written = format!("{}+{}", hotkeys.modifiers, hotkey.key);
        let binding = Binding::parse(&written).expect("a hotkey the config would accept");
        assert_eq!(
            binding,
            defaults.binding(action),
            "{written} for {action:?}"
        );
    }
}

#[test]
fn the_fastest_stable_interval_is_the_interval_floor() {
    let figures = facts().figures;
    let figure = figures
        .iter()
        .find(|f| f.id == "fastest-interval")
        .expect("docs/facts.json has the fastest-interval figure");
    assert_eq!(figure.unit, "ms");
    assert_eq!(figure.basis, Basis::Measured);
    let millis: u64 = figure.value.parse().expect("a whole number of ms");
    assert_eq!(Duration::from_millis(millis), INTERVAL_FLOOR);
}

#[test]
fn every_path_is_a_file_in_the_repository_and_every_quote_is_in_its_source() {
    let facts = facts();
    let notes = facts.platforms.iter().filter_map(|p| p.note.as_ref());
    let sources = facts.figures.iter().map(|f| &f.source.path);
    let paths = facts
        .links
        .values()
        .chain(notes.map(|n| &n.link))
        .chain(sources);
    for path in paths {
        assert!(
            is_file_in(&repository(), path),
            "docs/facts.json names {path}, which is not a file in the repository"
        );
    }
    for figure in &facts.figures {
        let source = fs::read_to_string(repository().join(&figure.source.path)).unwrap();
        assert!(
            source.contains(&figure.source.quote),
            "figure {}: {} does not contain {:?}",
            figure.id,
            figure.source.path,
            figure.source.quote
        );
    }
}

#[test]
fn a_path_is_a_file_only_with_no_link_along_it() {
    let root = std::env::temp_dir().join(format!("auto-clicker-facts-{}", std::process::id()));
    fs::create_dir_all(root.join("docs/guide")).unwrap();
    fs::write(root.join("docs/guide/install.md"), "x").unwrap();
    assert!(is_file_in(&root, "docs/guide/install.md"));
    assert!(!is_file_in(&root, "docs/guide/gone.md"));
    assert!(!is_file_in(&root, "docs/guide"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink("guide", root.join("docs/linked")).unwrap();
        symlink("install.md", root.join("docs/guide/linked.md")).unwrap();
        assert!(!is_file_in(&root, "docs/linked/install.md"));
        assert!(!is_file_in(&root, "docs/guide/linked.md"));
    }
    fs::remove_dir_all(&root).unwrap();
}
