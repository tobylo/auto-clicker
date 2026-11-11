//! The recordings list as the window shows it: one row per recording the
//! daemon listed, and which row is selected. A selection follows its
//! recording, by file, when the list changes, and ends when the recording
//! is gone.

use auto_clicker_core::controller::Listed;
use auto_clicker_core::recording::FileName;
use auto_clicker_core::window::RecordingsView;

/// One row: the name, when it was made, and whether the playback hotkey
/// plays it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// `None` when the name cannot be read.
    pub name: Option<String>,
    pub when: String,
    pub current: bool,
}

/// The rows for `view`.
pub fn rows(view: &RecordingsView) -> Vec<Row> {
    view.listed
        .iter()
        .map(|listed| Row {
            name: listed.name.clone(),
            when: listed.file.when(),
            current: view.current.as_ref() == Some(&listed.file),
        })
        .collect()
}

/// Where the recording `selected` is in `view`'s list, if it is there.
pub fn place(view: &RecordingsView, selected: Option<&FileName>) -> Option<usize> {
    let selected = selected?;
    view.listed
        .iter()
        .position(|listed| listed.file == *selected)
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
    fn rows_show_the_name_when_it_reads_and_mark_the_current_one() {
        let shown = rows(&view(&[10, 9], Some(9)));
        assert_eq!(
            shown,
            [
                Row {
                    name: Some("Day 10".to_owned()),
                    when: "2026-11-10 18:04:05".to_owned(),
                    current: false,
                },
                Row {
                    name: None,
                    when: "2026-11-09 18:04:05".to_owned(),
                    current: true,
                },
            ]
        );
        assert!(rows(&view(&[10], None)).iter().all(|row| !row.current));
    }

    #[test]
    fn a_selection_follows_its_recording_and_ends_when_it_is_gone() {
        let selected = file(9);
        assert_eq!(place(&view(&[10, 9], None), Some(&selected)), Some(1));
        assert_eq!(place(&view(&[11, 10, 9], None), Some(&selected)), Some(2));
        assert_eq!(place(&view(&[11, 10], None), Some(&selected)), None);
        assert_eq!(place(&view(&[9], None), None), None);
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
