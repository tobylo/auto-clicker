//! What README.md shows, read by a Markdown parser with GitHub's extensions:
//! the code blocks under a heading, and the hotkey table. Whatever form the
//! Markdown takes (fences, indented blocks, quotes, lists, tables without
//! outer pipes), these are the blocks and cells GitHub renders, provided the
//! README is plain Markdown (`plain`), which the tests also check.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// The fence languages GitHub shows as code. Others it may draw instead, as
/// a diagram, a map or mathematics.
const LITERAL: [&str; 7] = ["", "fish", "sh", "bash", "shell", "console", "text"];

fn events(markdown: &str) -> Parser<'_> {
    let gfm = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    Parser::new_ext(markdown, gfm)
}

/// Whether the README is plain Markdown after its icon: one HTML block
/// holding a single `<img>` tag may open it, and nothing else may be HTML,
/// an image, a footnote, or a fence in a language GitHub may draw (see
/// `is_plain`). In plain Markdown, what the parser reads as a block, a cell
/// or a heading is what GitHub shows there, so no container can hide,
/// move or add to the text the tests compare.
pub fn plain(markdown: &str) -> Result<(), String> {
    let events: Vec<_> = events(markdown).into_offset_iter().collect();
    let after_icon = match events.as_slice() {
        [
            (Event::Start(Tag::HtmlBlock), _),
            (Event::Html(html), _),
            (Event::End(TagEnd::HtmlBlock), _),
            rest @ ..,
        ] if is_icon(html) => rest,
        all => all,
    };
    match after_icon.iter().find(|(event, _)| !is_plain(event)) {
        Some((_, at)) => {
            let line = markdown[..at.start].matches('\n').count() + 1;
            Err(format!(
                "README.md line {line}: more than plain Markdown (HTML, an image, a footnote or \
                 a fence GitHub may draw); the facts tests read the README as plain Markdown"
            ))
        }
        None => Ok(()),
    }
}

/// One `<img ...>` tag on its own.
fn is_icon(html: &str) -> bool {
    let html = html.trim();
    html.starts_with("<img ") && html.ends_with('>') && html.matches(['<', '>']).count() == 2
}

/// Whether `event` is plain Markdown, which shows what it says where it says
/// it: text, code, emphasis, links, lists, quotes, code blocks, headings,
/// tables, rules and soft breaks. HTML can hide text or show text the parser
/// does not read as such, an image shows something other than its alt text,
/// a footnote's definition moves to the foot of the page and its reference
/// adds a mark to the text, and a hard break splits a line the tests read as
/// one.
fn is_plain(event: &Event<'_>) -> bool {
    match event {
        Event::Start(tag) => is_plain_tag(tag),
        Event::End(_)
        | Event::Text(_)
        | Event::Code(_)
        | Event::SoftBreak
        | Event::Rule
        | Event::TaskListMarker(_) => true,
        _ => false,
    }
}

fn is_plain_tag(tag: &Tag<'_>) -> bool {
    match tag {
        Tag::CodeBlock(CodeBlockKind::Fenced(info)) => LITERAL.contains(&info.as_ref()),
        _ => matches!(
            tag,
            Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::BlockQuote(_)
                | Tag::CodeBlock(CodeBlockKind::Indented)
                | Tag::List(_)
                | Tag::Item
                | Tag::Emphasis
                | Tag::Strong
                | Tag::Strikethrough
                | Tag::Link { .. }
                | Tag::Table(_)
                | Tag::TableHead
                | Tag::TableRow
                | Tag::TableCell
        ),
    }
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
    /// The heading being read: its level and its text so far.
    heading: Option<(HeadingLevel, String)>,
    /// The wanted section's level, while inside it.
    open: Option<HeadingLevel>,
    /// The code block being read.
    code: Option<String>,
    found: usize,
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
            // A heading over two lines (setext) reads as one line.
            Event::SoftBreak => self.text(" "),
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
}

#[cfg(test)]
mod tests {
    use super::{hotkey_column, plain, section_commands};

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
    fn reads_a_heading_over_two_lines_as_one() {
        let markdown = INSTALL.replace("## Install", "Install\nfrom source\n---");
        let commands = section_commands(&markdown, "Install from source").unwrap();
        assert_eq!(commands, ["a", "b"]);
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

    #[track_caller]
    fn not_plain(markdown: &str, line: usize) {
        let err = plain(markdown).unwrap_err();
        assert!(
            err.starts_with(&format!("README.md line {line}: more than plain")),
            "{err}"
        );
    }

    #[test]
    fn accepts_plain_markdown_after_an_icon() {
        plain(INSTALL).unwrap();
        plain(&format!(
            "<img src=\"i.svg\" alt=\"\">\n\n{INSTALL}{HOTKEYS}"
        ))
        .unwrap();
        let in_body =
            format!("{INSTALL}- [x] done\n\n---\n\n*a* **b** ~~c~~ [d](e)\n\n> f\n\n    g\n");
        plain(&in_body).unwrap();
    }

    #[test]
    fn refuses_more_than_plain_markdown_anywhere_else() {
        let cases = [
            ("a second icon", "<img src=\"i.svg\">"),
            ("a comment", "<!--\n```\nhidden\n```\n-->"),
            ("a block", "<pre>shown</pre>"),
            ("an inline tag", "Run <b>this</b>:"),
            ("a wrapper", "<details>\n\n```\nc\n```\n\n</details>"),
            ("an image", "![a](b.svg)"),
            ("a footnote", "Run[^1]:\n\n[^1]: note"),
            ("a hard break", "Run\\\nthis:"),
            ("a drawn fence", "```mermaid\ngraph\n```"),
            ("a math fence", "```math\nx\n```"),
        ];
        for (name, markdown) in cases {
            let err = plain(&INSTALL.replace("Run:", markdown)).expect_err(name);
            assert!(
                err.starts_with("README.md line 5: more than plain"),
                "{name}: {err}"
            );
        }
        not_plain(&format!("{INSTALL}\n<details>"), 19);
        not_plain(&format!("<details>\n\n<img src=\"i.svg\">\n\n{INSTALL}"), 1);
        not_plain(&format!("<img src=\"i.svg\"><b>x</b>\n\n{INSTALL}"), 1);
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
    }
}
