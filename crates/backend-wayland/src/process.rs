//! Who a process's parent is: whether a compositor's process belongs to
//! the display, as its listener or the listener's child (W01 brief, W1).
//! KWin's backend and Hyprland's use the same rule.

use std::num::NonZeroU32;

/// A process's parent, as `/proc` tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parent {
    Of(u32),
    /// The process is gone: it runs no display.
    Gone,
    /// Its status could not be read or parsed: not told.
    Unknown,
}

/// The parent of process `pid`, from `/proc/<pid>/status`.
pub fn parent_of(pid: u32) -> Parent {
    match std::fs::read_to_string(format!("/proc/{pid}/status")) {
        Ok(status) => parse_parent(&status).map_or(Parent::Unknown, Parent::Of),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Parent::Gone,
        Err(_) => Parent::Unknown,
    }
}

/// Whether process `pid`, whose parent is `parent`, is `listener` or its
/// child; `None` when it cannot be told: it is not the listener and its
/// parent is unknown.
pub fn listener_or_child(pid: u32, parent: Parent, listener: NonZeroU32) -> Option<bool> {
    let listener = listener.get();
    if pid == listener {
        return Some(true);
    }
    match parent {
        Parent::Of(parent) => Some(parent == listener),
        Parent::Gone => Some(false),
        Parent::Unknown => None,
    }
}

fn parse_parent(status: &str) -> Option<u32> {
    let line = status.lines().find_map(|line| line.strip_prefix("PPid:"))?;
    line.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parent_is_read_from_the_status_file() {
        let status = "Name:\tkwin_wayland\nState:\tS (sleeping)\nPid:\t1491\nPPid:\t1487\n";
        assert_eq!(parse_parent(status), Some(1487));
        assert_eq!(parse_parent("Name:\tx\nPPid:\tx\n"), None);
        assert_eq!(parse_parent("Name:\tx\n"), None);
        assert_eq!(
            parent_of(std::process::id()),
            Parent::Of(std::os::unix::process::parent_id())
        );
        // Past the kernel's largest pid: no such process.
        assert_eq!(parent_of(u32::MAX), Parent::Gone);
    }

    #[test]
    fn a_process_belongs_to_the_listener_as_itself_or_its_child() {
        let listener = NonZeroU32::new(1487).unwrap();
        let cases = [
            (1487, Parent::Of(1), Some(true)),
            (1487, Parent::Unknown, Some(true)),
            (1491, Parent::Of(1487), Some(true)),
            (2210, Parent::Of(2201), Some(false)),
            (2210, Parent::Gone, Some(false)),
            (2210, Parent::Unknown, None),
        ];
        for (pid, parent, belongs) in cases {
            assert_eq!(
                listener_or_child(pid, parent, listener),
                belongs,
                "{pid} {parent:?}"
            );
        }
    }
}
