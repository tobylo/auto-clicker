use super::*;

fn os(text: &str) -> Option<OsString> {
    Some(OsString::from(text))
}

/// Each case: the XDG variable, `HOME`, and the folder expected.
type Case = (Option<OsString>, Option<OsString>, Option<&'static str>);

fn check(fallback: &[&str], cases: &[Case]) {
    for (xdg, home, expected) in cases {
        assert_eq!(
            from(xdg.clone(), home.clone(), fallback),
            expected.map(PathBuf::from),
            "{xdg:?} {home:?}"
        );
    }
}

#[test]
fn the_config_folder_follows_xdg_config_home_then_home() {
    let fallback = Some("/home/me/.config/auto-clicker");
    check(
        &[".config"],
        &[
            (os("/xdg"), os("/home/me"), Some("/xdg/auto-clicker")),
            (None, os("/home/me"), fallback),
            (os(""), os("/home/me"), fallback),
            (os("relative"), os("/home/me"), fallback),
            (None, os("relative"), None),
            (None, None, None),
        ],
    );
}

#[test]
fn the_data_folder_follows_xdg_data_home_then_home() {
    let fallback = Some("/home/u/.local/share/auto-clicker");
    check(
        &[".local", "share"],
        &[
            (os("/data"), os("/home/u"), Some("/data/auto-clicker")),
            (None, os("/home/u"), fallback),
            (os("relative"), os("/home/u"), fallback),
            (None, os("relative"), None),
            (None, None, None),
        ],
    );
}

#[test]
fn the_state_folder_follows_xdg_state_home_then_home() {
    let fallback = Some("/home/u/.local/state/auto-clicker");
    check(
        &[".local", "state"],
        &[
            (os("/state"), os("/home/u"), Some("/state/auto-clicker")),
            (os("relative"), os("/home/u"), fallback),
            (None, os("/home/u"), fallback),
            (None, os("relative"), None),
            (None, None, None),
        ],
    );
}

#[test]
fn each_folder_reads_its_own_variables() {
    let home = var("HOME");
    let read = |name, fallback: &[&str]| from(var(name), home.clone(), fallback);
    assert_eq!(config_dir(), read("XDG_CONFIG_HOME", &[".config"]));
    assert_eq!(data_dir(), read("XDG_DATA_HOME", &[".local", "share"]));
    assert_eq!(state_dir(), read("XDG_STATE_HOME", &[".local", "state"]));
}
