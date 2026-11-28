use std::ffi::{OsStr, OsString};

use super::*;

fn split(line: &[u16]) -> String {
    String::from_utf16(&line[..line.len() - 1]).unwrap()
}

#[test]
fn the_command_line_quotes_what_needs_it_as_the_c_runtime_reads_it() {
    let arguments: Vec<OsString> = [
        "--table",
        "1234",
        "a b",
        "",
        r#"say "hi""#,
        r"C:\dir\",
        r"C:\a b\",
        r"x\\y",
        r#"a\"b c"#,
    ]
    .iter()
    .map(OsString::from)
    .collect();
    // Backslashes count only before a quote: doubled there, kept elsewhere.
    assert_eq!(
        split(&command_line(
            OsStr::new(r"C:\Program Files\a.exe"),
            &arguments
        )),
        r#""C:\Program Files\a.exe" --table 1234 "a b" "" "say \"hi\"" C:\dir\ "C:\a b\\" x\\y "a\\\"b c""#
    );
}
