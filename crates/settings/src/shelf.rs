//! The recordings list as the window shows it: one row per recording the
//! daemon listed, and which rows are selected. A selection follows its
//! recordings, by file, when the list changes, and drops each one gone.

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

/// How a click on a row selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    /// That row alone: a plain click.
    Only,
    /// That row in or out, the others as they are: Ctrl+click.
    Toggle,
    /// The rows from the last plain or Ctrl click to that one: Shift+click.
    Range,
}

/// The selected recordings, by file, in the order the list shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    files: Vec<FileName>,
    /// Where a Shift range starts: the row last clicked without Shift, or
    /// moved to without it.
    anchor: Option<FileName>,
    /// The row the arrow keys move from: the one last clicked or moved to.
    cursor: Option<FileName>,
}

impl Selection {
    /// The selected recordings, in list order.
    pub fn files(&self) -> &[FileName] {
        &self.files
    }

    /// The recording selected, when exactly one is.
    pub fn one(&self) -> Option<&FileName> {
        match &self.files[..] {
            [one] => Some(one),
            _ => None,
        }
    }

    /// Whether each row of `view` is selected.
    pub fn flags(&self, view: &RecordingsView) -> Vec<bool> {
        view.listed
            .iter()
            .map(|listed| self.files.contains(&listed.file))
            .collect()
    }

    /// Where the row the arrow keys move from is, as Slint takes it: -1
    /// for none.
    pub fn cursor(&self, view: &RecordingsView) -> i32 {
        slint_place(
            self.cursor
                .as_ref()
                .and_then(|cursor| position(view, cursor)),
        )
    }

    /// The row at `index` was clicked, as `click` says; a place that holds
    /// no row changes nothing.
    pub fn click(&mut self, view: &RecordingsView, index: i32, click: Click) {
        let Some(listed) = at(view, index) else {
            return;
        };
        let file = listed.file.clone();
        match click {
            Click::Only => self.files = vec![file.clone()],
            Click::Toggle => {
                let mut files = std::mem::take(&mut self.files);
                if files.contains(&file) {
                    files.retain(|selected| *selected != file);
                } else {
                    files.push(file.clone());
                }
                self.files = in_list_order(view, &files);
            }
            Click::Range => {
                self.range(view, &file);
                self.cursor = Some(file);
                return;
            }
        }
        self.anchor = Some(file.clone());
        self.cursor = Some(file);
    }

    /// The arrow keys moved the selection `by` rows (Home and End by as
    /// many as there are), selecting that row alone or, with Shift
    /// (`extend`), the rows from the anchor to it. With no row to move
    /// from, it moves from just above the first: Down and Home reach the
    /// first, End the last.
    pub fn step(&mut self, view: &RecordingsView, by: i32, extend: bool) {
        let Some(last) = view.listed.len().checked_sub(1) else {
            return;
        };
        let by = isize::try_from(by).unwrap_or(isize::MAX);
        let to = match self
            .cursor
            .as_ref()
            .and_then(|cursor| position(view, cursor))
        {
            Some(from) => from.saturating_add_signed(by),
            None => usize::try_from(by.saturating_sub(1)).unwrap_or(0),
        }
        .min(last);
        let file = view.listed[to].file.clone();
        if extend {
            self.range(view, &file);
        } else {
            self.files = vec![file.clone()];
            self.anchor = Some(file.clone());
        }
        self.cursor = Some(file);
    }

    /// Ctrl+A: every listed recording.
    pub fn all(&mut self, view: &RecordingsView) {
        self.files = view
            .listed
            .iter()
            .map(|listed| listed.file.clone())
            .collect();
    }

    /// The list changed to `view`: keeps what is still listed, in its new
    /// order.
    pub fn follow(&mut self, view: &RecordingsView) {
        let listed = |file: &Option<FileName>| {
            file.as_ref()
                .filter(|file| position(view, file).is_some())
                .cloned()
        };
        self.anchor = listed(&self.anchor);
        self.cursor = listed(&self.cursor);
        self.files = in_list_order(view, &self.files);
    }

    /// Selects the rows from the anchor, or the first row with none, to
    /// `to`, and only those.
    fn range(&mut self, view: &RecordingsView, to: &FileName) {
        let from = self
            .anchor
            .as_ref()
            .and_then(|anchor| position(view, anchor))
            .unwrap_or(0);
        let to = position(view, to).unwrap_or(from);
        let (first, last) = (from.min(to), from.max(to));
        self.files = view.listed[first..=last]
            .iter()
            .map(|listed| listed.file.clone())
            .collect();
    }
}

/// Those of `files` that `view` lists, in its order.
fn in_list_order(view: &RecordingsView, files: &[FileName]) -> Vec<FileName> {
    view.listed
        .iter()
        .filter(|listed| files.contains(&listed.file))
        .map(|listed| listed.file.clone())
        .collect()
}

/// Where `file` is in `view`'s list.
fn position(view: &RecordingsView, file: &FileName) -> Option<usize> {
    view.listed.iter().position(|listed| listed.file == *file)
}

/// `place` as Slint takes it: -1 for none.
fn slint_place(place: Option<usize>) -> i32 {
    place
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

    /// The days of the selected recordings.
    fn days(selection: &Selection) -> Vec<u32> {
        selection
            .files()
            .iter()
            .map(|file| file.as_str()[8..10].parse().unwrap())
            .collect()
    }

    #[test]
    fn a_click_selects_one_ctrl_toggles_and_shift_takes_the_range_from_the_last_click() {
        let list = view(&[15, 14, 13, 12, 11, 10], None);
        let mut selection = Selection::default();

        selection.click(&list, 1, Click::Only);
        assert_eq!(days(&selection), [14]);
        assert_eq!(selection.one(), Some(&file(14)));
        selection.click(&list, 3, Click::Toggle);
        assert_eq!(days(&selection), [14, 12]);
        assert_eq!(selection.one(), None);
        // From the Ctrl+click, up or down, replacing the rest.
        selection.click(&list, 5, Click::Range);
        assert_eq!(days(&selection), [12, 11, 10]);
        selection.click(&list, 0, Click::Range);
        assert_eq!(days(&selection), [15, 14, 13, 12]);
        // Ctrl+click again takes one out, and the last one out leaves none.
        selection.click(&list, 0, Click::Toggle);
        assert_eq!(days(&selection), [14, 13, 12]);
        selection.click(&list, 4, Click::Only);
        selection.click(&list, 4, Click::Toggle);
        assert!(selection.files().is_empty());
        assert_eq!(selection.one(), None);
        // A Shift+click with no click before ranges from the first row.
        let mut fresh = Selection::default();
        fresh.click(&list, 2, Click::Range);
        assert_eq!(days(&fresh), [15, 14, 13]);
        assert_eq!(
            selection.flags(&list),
            [false, false, false, false, false, false]
        );
        fresh.click(&list, 1, Click::Toggle);
        assert_eq!(fresh.flags(&list), [true, false, true, false, false, false]);
    }

    #[test]
    fn a_click_off_the_rows_changes_nothing() {
        let list = view(&[10, 9], None);
        let mut selection = Selection::default();
        selection.click(&list, 0, Click::Only);
        let before = selection.clone();
        for index in [-1, 2, i32::MAX, i32::MIN] {
            for click in [Click::Only, Click::Toggle, Click::Range] {
                selection.click(&list, index, click);
                assert_eq!(selection, before, "{index} {click:?}");
            }
        }
    }

    #[test]
    fn the_arrows_move_one_row_shift_extends_and_home_and_end_reach_the_ends() {
        let list = view(&[15, 14, 13, 12, 11], None);
        let mut selection = Selection::default();
        // With nothing to move from, the first row.
        selection.step(&list, 1, false);
        assert_eq!(days(&selection), [15]);
        selection.step(&list, 1, false);
        selection.step(&list, 1, false);
        assert_eq!(days(&selection), [13]);
        assert_eq!(selection.cursor(&list), 2);
        selection.step(&list, -1, true);
        selection.step(&list, -1, true);
        assert_eq!(days(&selection), [15, 14, 13]);
        assert_eq!(selection.cursor(&list), 0);
        // Past the ends: there.
        selection.step(&list, -1, true);
        assert_eq!(days(&selection), [15, 14, 13]);
        selection.step(&list, i32::MAX, true);
        assert_eq!(days(&selection), [13, 12, 11]);
        selection.step(&list, i32::MIN, false);
        assert_eq!(days(&selection), [15]);
        // A Shift+click ranges from where the arrows left the anchor.
        selection.step(&list, 2, false);
        selection.click(&list, 4, Click::Range);
        assert_eq!(days(&selection), [13, 12, 11]);
        assert_eq!(selection.cursor(&list), 4);
        let mut none = Selection::default();
        none.step(&view(&[], None), 1, false);
        assert_eq!(none, Selection::default());
        // With no row to move from, End reaches the last and Up the first.
        let mut ended = Selection::default();
        ended.step(&list, 5, false);
        assert_eq!(days(&ended), [11]);
        let mut up = Selection::default();
        up.step(&list, -1, false);
        assert_eq!(days(&up), [15]);
    }

    #[test]
    fn ctrl_a_selects_every_listed_recording() {
        let list = view(&[12, 11, 10], None);
        let mut selection = Selection::default();
        selection.all(&list);
        assert_eq!(days(&selection), [12, 11, 10]);
        selection.all(&view(&[], None));
        assert!(selection.files().is_empty());
    }

    #[test]
    fn a_selection_follows_its_recordings_and_drops_each_one_gone() {
        let list = view(&[12, 11, 10, 9], None);
        let mut selection = Selection::default();
        selection.click(&list, 1, Click::Only);
        selection.click(&list, 3, Click::Toggle);
        assert_eq!(selection.cursor(&list), 3);

        let newer = view(&[13, 12, 11, 10, 9], None);
        selection.follow(&newer);
        assert_eq!(days(&selection), [11, 9]);
        assert_eq!(selection.cursor(&newer), 4);

        let fewer = view(&[13, 11, 10], None);
        selection.follow(&fewer);
        assert_eq!(days(&selection), [11]);
        assert_eq!(selection.cursor(&fewer), -1);
        // The anchor, Day 9, went too: a range starts at the first row.
        selection.click(&fewer, 1, Click::Range);
        assert_eq!(days(&selection), [13, 11]);
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
