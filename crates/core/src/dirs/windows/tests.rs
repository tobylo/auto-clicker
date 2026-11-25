use super::*;

#[test]
fn messages_name_the_variables_that_were_missing() {
    assert_eq!(NO_CONFIG_DIR, "APPDATA is not set to an absolute folder");
    assert_eq!(NO_DATA_DIR, "LOCALAPPDATA is not set to an absolute folder");
}

#[test]
fn a_folder_needs_an_absolute_path() {
    let os = |text: &str| Some(OsString::from(text));
    for (dir, expected) in [
        (
            os(r"C:\Users\me\AppData\Roaming"),
            Some(r"C:\Users\me\AppData\Roaming\auto-clicker"),
        ),
        // A folder redirected to a server.
        (
            os(r"\\server\share\me\AppData"),
            Some(r"\\server\share\me\AppData\auto-clicker"),
        ),
        (os(r"\Users\me\AppData"), None),
        (os(r"C:Users\me"), None),
        (os("relative"), None),
        (os(""), None),
        (None, None),
    ] {
        assert_eq!(from(dir.clone()), expected.map(PathBuf::from), "{dir:?}");
    }
}

#[test]
fn config_roams_and_the_rest_stays_on_this_machine() {
    let read = |name| from(std::env::var_os(name));
    assert_eq!(config_dir(), read("APPDATA"));
    assert_eq!(data_dir(), read("LOCALAPPDATA"));
    assert_eq!(state_dir(), read("LOCALAPPDATA"));
}
