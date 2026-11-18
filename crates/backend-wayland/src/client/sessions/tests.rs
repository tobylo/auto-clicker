use super::{CAPTURE_MANAGER, Global, Offered, SEAT, SOURCE_MANAGER};

fn global(name: u32) -> Global {
    Global { name, version: 1 }
}

#[test]
fn cursor_sessions_need_all_three_globals_while_each_is_offered() {
    let mut offered = Offered::default();
    offered.note(SEAT, global(1));
    offered.note(CAPTURE_MANAGER, global(2));
    assert!(!offered.all(), "no source manager");
    offered.note("wl_output", global(3));
    assert!(!offered.all(), "an output is none of them");
    offered.note(SOURCE_MANAGER, global(4));
    assert!(offered.all());
    offered.removed(3);
    assert!(offered.all(), "another global went");
    for (name, interface) in [(1, SEAT), (2, CAPTURE_MANAGER), (4, SOURCE_MANAGER)] {
        offered.removed(name);
        assert!(!offered.all(), "{interface} went");
        offered.note(interface, global(name));
        assert!(offered.all(), "{interface} came back");
    }
}
