//! What README.md shows, read by a Markdown parser with GitHub's extensions:
//! the code blocks under a heading, and the hotkey table. Whatever form the
//! Markdown takes (fences, indented blocks, quotes, lists, tables without
//! outer pipes), these are the blocks and cells GitHub renders. HTML is
//! refused where the check reads, since it can hide text or show some the
//! parser does not see as a block.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

fn events(markdown: &str) -> Parser<'_> {
    let gfm = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    Parser::new_ext(markdown, gfm)
}

/// The non-blank lines of every code block in the section under the one
/// heading whose text is `heading`: up to the next heading of its level or
/// higher, its subsections included.
pub fn section_commands(markdown: &str, heading: &str) -> Result<Vec<String>, String> {
    let mut section = Section {
        want: heading,
        ..Section::default()
    };
    for event in events(markdown) {
        section.take(event);
    }
    section.finish()
}

#[derive(Default)]
struct Section<'w> {
    want: &'w str,
    /// The heading being read: its level and text so far.
    heading: Option<(HeadingLevel, String)>,
    /// The wanted section's level, while inside it.
    open: Option<HeadingLevel>,
    /// The code block being read.
    code: Option<String>,
    found: usize,
    html: bool,
    lines: Vec<String>,
}

impl Section<'_> {
    fn take(&mut self, event: Event<'_>) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => self.start_heading(level),
            Event::End(TagEnd::Heading(_)) => self.end_heading(),
            Event::Start(Tag::CodeBlock(_)) => self.code = Some(String::new()),
            Event::End(TagEnd::CodeBlock) => self.end_code(),
            Event::Text(text) | Event::Code(text) => self.text(&text),
            Event::Html(_) | Event::InlineHtml(_) => self.html |= self.open.is_some(),
            _ => {}
        }
    }

    fn start_heading(&mut self, level: HeadingLevel) {
        if self.open.is_some_and(|open| level <= open) {
            self.open = None;
        }
        self.heading = Some((level, String::new()));
    }

    fn end_heading(&mut self) {
        if let Some((level, text)) = self.heading.take()
            && text == self.want
        {
            self.found += 1;
            self.open = Some(level);
        }
    }

    fn text(&mut self, text: &str) {
        if let Some((_, heading)) = &mut self.heading {
            heading.push_str(text);
        } else if let Some(code) = &mut self.code {
            code.push_str(text);
        }
    }

    fn end_code(&mut self) {
        let Some(code) = self.code.take() else { return };
        if self.open.is_some() {
            let lines = code.lines().filter(|line| !line.trim().is_empty());
            self.lines.extend(lines.map(str::to_owned));
        }
    }

    fn finish(self) -> Result<Vec<String>, String> {
        if self.found != 1 {
            return Err(format!(
                "README.md has {} headings \"{}\"; it needs exactly one",
                self.found, self.want
            ));
        }
        if self.html {
            return Err(format!(
                "README.md has HTML under \"{}\"; write that section in Markdown only",
                self.want
            ));
        }
        Ok(self.lines)
    }
}

/// The first cell of each row of the one table headed "Hotkey | Does".
pub fn hotkey_column(markdown: &str) -> Result<Vec<String>, String> {
    let tables = tables(markdown);
    let hotkeys: Vec<&Table> = tables
        .iter()
        .filter(|t| t.head == ["Hotkey", "Does"])
        .collect();
    match hotkeys.as_slice() {
        [table] if table.html => {
            Err("README.md's hotkey table has HTML; write it in Markdown only".into())
        }
        [table] => Ok(table
            .rows
            .iter()
            .map(|row| row.first().cloned().unwrap_or_default())
            .collect()),
        _ => Err(format!(
            "README.md has {} tables headed \"Hotkey | Does\"; it needs exactly one",
            hotkeys.len()
        )),
    }
}

#[derive(Default)]
struct Table {
    head: Vec<String>,
    rows: Vec<Vec<String>>,
    html: bool,
}

/// Every table, with each cell's text.
fn tables(markdown: &str) -> Vec<Table> {
    let mut scan = Tables::default();
    for event in events(markdown) {
        scan.take(event);
    }
    scan.done
}

#[derive(Default)]
struct Tables {
    done: Vec<Table>,
    table: Option<Table>,
    in_head: bool,
    cell: Option<String>,
}

impl Tables {
    fn take(&mut self, event: Event<'_>) {
        match event {
            Event::Start(Tag::Table(_)) => self.table = Some(Table::default()),
            Event::End(TagEnd::Table) => self.done.extend(self.table.take()),
            Event::Start(Tag::TableHead) => self.in_head = true,
            Event::End(TagEnd::TableHead) => self.in_head = false,
            Event::Start(Tag::TableRow) => self.start_row(),
            Event::Start(Tag::TableCell) => self.cell = Some(String::new()),
            Event::End(TagEnd::TableCell) => self.end_cell(),
            Event::Text(text) | Event::Code(text) => self.text(&text),
            Event::Html(_) | Event::InlineHtml(_) => self.html(),
            _ => {}
        }
    }

    fn start_row(&mut self) {
        if let Some(table) = &mut self.table {
            table.rows.push(Vec::new());
        }
    }

    fn end_cell(&mut self) {
        let (Some(table), Some(cell)) = (&mut self.table, self.cell.take()) else {
            return;
        };
        if self.in_head {
            table.head.push(cell);
        } else if let Some(row) = table.rows.last_mut() {
            row.push(cell);
        }
    }

    fn text(&mut self, text: &str) {
        if let Some(cell) = &mut self.cell {
            cell.push_str(text);
        }
    }

    fn html(&mut self) {
        if let Some(table) = &mut self.table {
            table.html = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{hotkey_column, section_commands};

    const INSTALL: &str =
        "# Title\n\n## Install\n\nRun:\n\n```fish\na\n\nb\n```\n\n## Next\n\n```\nnot this\n```\n";

    #[track_caller]
    fn commands(markdown: &str) -> Vec<String> {
        section_commands(markdown, "Install").unwrap()
    }

    #[track_caller]
    fn refused(markdown: &str, problem: &str) {
        let err = section_commands(markdown, "Install").unwrap_err();
        assert!(err.contains(problem), "expected {problem:?}, got {err:?}");
    }

    #[test]
    fn reads_the_lines_of_the_blocks_in_the_section_only() {
        assert_eq!(commands(INSTALL), ["a", "b"]);
    }

    #[test]
    fn reads_every_form_of_code_block() {
        let forms = [
            ("tilde fence", "~~~\nc\n~~~\n"),
            ("long fence", "````\nc\n````\n"),
            ("indented", "    c\n"),
            ("indented fence", "  ```\n  c\n  ```\n"),
            ("in a quote", "> ```\n> c\n> ```\n"),
            ("in a list", "- step:\n\n  ```\n  c\n  ```\n"),
            ("indented in a quote", ">     c\n"),
        ];
        for (name, block) in forms {
            let markdown = INSTALL.replace("Run:\n", &format!("Run:\n\n{block}"));
            assert_eq!(commands(&markdown), ["c", "a", "b"], "{name}");
        }
    }

    #[test]
    fn a_comment_in_a_block_is_not_a_heading() {
        let markdown = INSTALL.replace("a\n\nb", "# a comment\nb");
        assert_eq!(commands(&markdown), ["# a comment", "b"]);
    }

    #[test]
    fn reads_subsections_and_stops_at_the_next_heading_of_its_level() {
        let markdown = INSTALL.replace("## Next", "### Sub\n\n```\nc\n```\n\n# Top");
        assert_eq!(commands(&markdown), ["a", "b", "c"]);
    }

    #[test]
    fn knows_every_form_of_heading() {
        for heading in ["Install\n-------", "## Install ##", "##   Install"] {
            let markdown = INSTALL.replace("## Install", heading);
            assert_eq!(commands(&markdown), ["a", "b"], "{heading}");
        }
    }

    #[test]
    fn needs_exactly_one_heading() {
        refused(
            &INSTALL.replace("## Next", "## Install"),
            "has 2 headings \"Install\"",
        );
        refused(
            &INSTALL.replace("## Install", "## Installing"),
            "has 0 headings",
        );
        refused(
            &INSTALL.replace("## Next", "## `Install`"),
            "has 2 headings",
        );
        refused(&INSTALL.replace("## Install", "#Install"), "has 0 headings");
    }

    #[test]
    fn refuses_html_in_the_section() {
        refused(
            &INSTALL.replace("Run:", "<!--\n```\nhidden\n```\n-->"),
            "HTML under \"Install\"",
        );
        refused(
            &INSTALL.replace("Run:", "<pre>shown</pre>"),
            "HTML under \"Install\"",
        );
        refused(
            &INSTALL.replace("Run:", "Run <!-- x -->:"),
            "HTML under \"Install\"",
        );
        let before = format!("<!-- a comment -->\n\n{INSTALL}<details></details>\n");
        assert_eq!(commands(&before), ["a", "b"]);
    }

    const HOTKEYS: &str = "| Hotkey | Does |\n| --- | --- |\n| `A` | x |\n| B | y |\n";

    #[test]
    fn reads_the_hotkey_column_as_shown() {
        assert_eq!(hotkey_column(HOTKEYS).unwrap(), ["A", "B"]);
        let no_outer_pipes = "Hotkey | Does\n--- | ---\nA | x\nB\n";
        assert_eq!(hotkey_column(no_outer_pipes).unwrap(), ["A", "B"]);
        let after_text = format!("{HOTKEYS}| C | z |\nnot a row\n\n| D | w |\n");
        assert_eq!(
            hotkey_column(&after_text).unwrap(),
            ["A", "B", "C", "not a row"]
        );
    }

    #[test]
    fn needs_exactly_one_hotkey_table_in_markdown() {
        let err = |markdown: &str| hotkey_column(markdown).unwrap_err();
        assert!(err(&format!("{HOTKEYS}\n{HOTKEYS}")).contains("has 2 tables"));
        assert!(
            err("<!--\n| Hotkey | Does |\n| --- | --- |\n| A | x |\n-->\n")
                .contains("has 0 tables")
        );
        assert!(err(&HOTKEYS.replace("| B |", "| <b>B</b> |")).contains("has HTML"));
    }
}
