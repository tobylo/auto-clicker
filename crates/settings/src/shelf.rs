//! The recordings list as the window shows it: one row per recording the
//! daemon listed, and which row is selected. A selection follows its
//! recording, by file, when the list changes, and ends when the recording
//! is gone.

use auto_clicker_core::controller::Listed;
use auto_clicker_core::recording::{FileName, MAX_NAME_CHARS};
use auto_clicker_core::window::RecordingsView;

/// One line for each listed recording: a ✓ on the one the playback hotkey
/// plays, its name (`no_name` when that cannot be read) and when it was
/// made.
pub fn lines(view: &RecordingsView, no_name: &str) -> Vec<String> {
    view.listed
        .iter()
        .map(|listed| {
            let mark = if view.current.as_ref() == Some(&listed.file) {
                "✓ "
            } else {
                ""
            };
            let name = listed.name.as_deref().unwrap_or(no_name);
            format!("{mark}{name} — {}", listed.file.when())
        })
        .collect()
}

/// `text`, typed as a new name, cut to one character more than a name may
/// have: the daemon refuses it and says why, and the request always fits a
/// line, however much was pasted.
pub fn sendable_name(text: &str) -> String {
    text.chars()
        .take(MAX_NAME_CHARS.saturating_add(1))
        .collect()
}

/// Where the recording `selected` is in `view`'s list, as Slint takes it:
/// -1 when it is not there.
pub fn place(view: &RecordingsView, selected: Option<&FileName>) -> i32 {
    selected
        .and_then(|selected| view.listed.iter().position(|l| l.file == *selected))
        .and_then(|place| i32::try_from(place).ok())
        .unwrap_or(-1)
}

/// What the list calls `file`, as a question about it names it.
pub fn label(view: &RecordingsView, file: &FileName) -> String {
    view.listed
        .iter()
        .find(|listed| listed.file == *file)
        .map_or_else(|| file.label(), |listed| listed.label.clone())
}

/// The recording at `index` in `view`'s list; `None` for a place that holds
/// none, such as Slint's -1.
pub fn at(view: &RecordingsView, index: i32) -> Option<&Listed> {
    view.listed.get(usize::try_from(index).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(day: u32) -> FileName {
        FileName::parse(&format!("2026-11-{day:02}_18-04-05.123.json")).unwrap()
    }

    fn view(days: &[u32], current: Option<u32>) -> RecordingsView {
        RecordingsView {
            listed: days
                .iter()
                .map(|&day| Listed::new(file(day), (day % 2 == 0).then(|| format!("Day {day}"))))
                .collect(),
            more: 0,
            current: current.map(file),
        }
    }

    #[test]
    fn lines_show_the_name_when_it_reads_and_mark_the_current_one() {
        assert_eq!(
            lines(&view(&[10, 9], Some(9)), "(no name)"),
            [
                "Day 10 — 2026-11-10 18:04:05",
                "✓ (no name) — 2026-11-09 18:04:05",
            ]
        );
        assert!(
            lines(&view(&[10], None), "-")
                .iter()
                .all(|line| !line.starts_with('✓'))
        );
    }

    #[test]
    fn a_name_is_sent_cut_just_past_what_a_recording_may_have() {
        assert_eq!(sendable_name("Farm loop"), "Farm loop");
        let long = "å".repeat(1_000_000);
        assert_eq!(sendable_name(&long).chars().count(), MAX_NAME_CHARS + 1);
    }

    #[test]
    fn a_selection_follows_its_recording_and_ends_when_it_is_gone() {
        let selected = file(9);
        assert_eq!(place(&view(&[10, 9], None), Some(&selected)), 1);
        assert_eq!(place(&view(&[11, 10, 9], None), Some(&selected)), 2);
        assert_eq!(place(&view(&[11, 10], None), Some(&selected)), -1);
        assert_eq!(place(&view(&[9, 8], None), None), -1);
    }

    #[test]
    fn a_question_names_the_recording_it_is_about() {
        let listed = view(&[10, 9], None);
        assert_eq!(label(&listed, &file(10)), "Day 10 (2026-11-10 18:04:05)");
        assert_eq!(label(&listed, &file(9)), "Recording 2026-11-09 18:04:05");
        assert_eq!(label(&listed, &file(8)), "Recording 2026-11-08 18:04:05");
    }

    #[test]
    fn only_a_listed_place_holds_a_recording() {
        let listed = view(&[10, 9], None);
        assert_eq!(at(&listed, 1).map(|l| l.file.clone()), Some(file(9)));
        for index in [-1, 2, i32::MAX, i32::MIN] {
            assert_eq!(at(&listed, index), None, "{index}");
        }
    }
}
