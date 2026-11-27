use std::sync::mpsc;
use std::time::Instant;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CMONITORS, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
};

use super::*;
use crate::become_per_monitor_aware;
use crate::desktop::{DisplayMode, display_sizes, screen_size};
use crate::message_thread::tests::{ARRIVAL, JOIN, send, started};

fn monitor([left, top, right, bottom]: [i32; 4], dpi: u32) -> Monitor {
    Monitor {
        left,
        top,
        right,
        bottom,
        dpi,
    }
}

/// The layout's outputs as (x, y, width, height, scale).
fn outputs(layout: &Layout) -> Vec<(u32, u32, u32, u32, u32)> {
    layout
        .outputs()
        .iter()
        .map(|output| {
            let Rect {
                x,
                y,
                width,
                height,
            } = output.rect();
            (x, y, width, height, output.scale_milli())
        })
        .collect()
}

#[test]
fn monitors_are_counted_from_the_virtual_desktops_top_left_corner() {
    let cases = [
        (
            vec![monitor([0, 0, 1920, 1080], 96)],
            vec![(0, 0, 1920, 1080, 1000)],
        ),
        // A monitor left of and above the primary one.
        (
            vec![
                monitor([0, 0, 1920, 1080], 96),
                monitor([-1280, -200, 0, 824], 120),
            ],
            vec![(0, 0, 1280, 1024, 1250), (1280, 200, 1920, 1080, 1000)],
        ),
        // Right of it, lower down: the corner is the primary's.
        (
            vec![
                monitor([0, 0, 2560, 1440], 144),
                monitor([2560, 300, 4480, 1380], 96),
            ],
            vec![(0, 0, 2560, 1440, 1500), (2560, 300, 1920, 1080, 1000)],
        ),
        // Mirrored: the same rectangle twice.
        (
            vec![
                monitor([0, 0, 1920, 1080], 96),
                monitor([0, 0, 1920, 1080], 96),
            ],
            vec![(0, 0, 1920, 1080, 1000), (0, 0, 1920, 1080, 1000)],
        ),
    ];
    for (monitors, expected) in cases {
        let layout = layout_of(&monitors).unwrap();
        assert_eq!(outputs(&layout), expected, "{monitors:?}");
    }
}

#[test]
fn a_monitors_scale_is_its_dpi_over_96_in_thousandths() {
    let cases = [
        (96, Some(1000)),
        (120, Some(1250)),
        (144, Some(1500)),
        (168, Some(1750)),
        (192, Some(2000)),
        // To the nearest thousandth.
        (97, Some(1010)),
        (0, None),
        // Past what a scale holds.
        (u32::MAX, None),
    ];
    for (dpi, scale) in cases {
        assert_eq!(scale_milli(dpi), scale, "{dpi}");
    }
}

#[test]
fn monitors_a_layout_cannot_hold_are_not_whole() {
    let one = monitor([0, 0, 1920, 1080], 96);
    let cases = [
        ("none", Vec::new()),
        ("seventeen", vec![one; 17]),
        ("no width", vec![monitor([0, 0, 0, 1080], 96)]),
        ("no height", vec![monitor([0, 0, 1920, 0], 96)]),
        ("inside out", vec![monitor([1920, 0, 0, 1080], 96)]),
        ("no DPI", vec![monitor([0, 0, 1920, 1080], 0)]),
    ];
    for (label, monitors) in cases {
        assert_eq!(layout_of(&monitors), Err(NoScreens::NotWhole), "{label}");
    }
    assert!(layout_of(&vec![one; 16]).is_ok());
    // Windows' coordinates always fit: its whole range is a layout's.
    let widest = [
        monitor([i32::MIN, 0, i32::MIN + 1, 1], 96),
        monitor([i32::MAX - 1, 0, i32::MAX, 1], 96),
    ];
    assert!(layout_of(&widest).is_ok());
}

/// The next event that is a screens change or a wake by `message`.
fn next_display_event(received: &mpsc::Receiver<WindowEvent>, message: u32) -> String {
    let deadline = Instant::now() + ARRIVAL;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match received.recv_timeout(left).unwrap() {
            WindowEvent::ScreensChanged => return "ScreensChanged".to_owned(),
            WindowEvent::Other(woken) if woken == message => return "Other".to_owned(),
            _ => {}
        }
    }
}

/// Waits for the screens to be told changed; the wakes of display changes
/// that left them as they were may come first.
fn changed(received: &mpsc::Receiver<WindowEvent>) {
    while next_display_event(received, WM_DISPLAYCHANGE) != "ScreensChanged" {}
}

/// The bounding box of `layout`'s outputs.
fn size(layout: &Layout) -> (u32, u32) {
    let right = layout.outputs().iter().map(|o| o.rect().x + o.rect().width);
    let bottom = layout
        .outputs()
        .iter()
        .map(|o| o.rect().y + o.rect().height);
    (right.max().unwrap_or(0), bottom.max().unwrap_or(0))
}

/// Hands the thread's window `message` as Windows does: `WM_DPICHANGED`
/// only synchronously, with the window's suggested rectangle, since Windows
/// refuses to post a message that carries a pointer.
fn tell(remote: &Remote, message: u32) {
    if message == WM_DPICHANGED {
        let suggested = RECT {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        };
        send(remote, message, 0, (&raw const suggested) as LPARAM).unwrap();
    } else {
        remote.post(message, 0, 0).unwrap();
    }
}

#[test]
fn a_display_change_reads_the_screens_again_and_tells_only_a_change() {
    let (thread, received) = started();
    let remote = thread.remote();
    let real = remote.screens();
    for message in [WM_DISPLAYCHANGE, WM_DPICHANGED] {
        // Kept as something else, so the next reading differs from it.
        let other = match real {
            Ok(_) => Err(NoScreens::NotWhole),
            Err(_) => layout_of(&[monitor([0, 0, 640, 480], 96)]),
        };
        remote.keep_screens(other);
        tell(&remote, message);
        assert_eq!(next_display_event(&received, message), "ScreensChanged");
        assert_eq!(remote.screens(), real);
        // The same screens again: only a wake, named as one.
        tell(&remote, message);
        assert_eq!(next_display_event(&received, message), "Other");
        assert_eq!(remote.screens(), real);
    }
    thread.quit_and_join(JOIN).unwrap();
}

/// The screens read are the virtual desktop in physical pixels: as many
/// outputs as monitors, and a bounding box of the virtual desktop's size.
/// The VM has one 1920x1080 monitor at 100 %, the PC one at 125 %; a
/// process that is not per-monitor aware would read the PC's as 1536x864.
#[test]
#[ignore = "reads the monitors of a desktop: run in the WN01 VM and on the PC"]
fn the_screens_are_the_monitors_in_physical_pixels() {
    // Set once per process; a second test setting it again is refused.
    let _ = become_per_monitor_aware();
    let layout = read().unwrap();
    let metric = |index| u32::try_from(system_metric(index)).unwrap();
    assert_eq!(
        size(&layout),
        (metric(SM_CXVIRTUALSCREEN), metric(SM_CYVIRTUALSCREEN)),
        "{layout:?}"
    );
    assert_eq!(
        layout.outputs().len(),
        usize::try_from(metric(SM_CMONITORS)).unwrap()
    );
    println!("screens: {:?}", outputs(&layout));
}

#[allow(unsafe_code)]
fn system_metric(index: i32) -> i32 {
    // SAFETY: plain call.
    unsafe { GetSystemMetrics(index) }
}

/// A new display mode reaches the message thread as a display change, its
/// screens are read again, and the mode put back is read back too.
#[test]
#[ignore = "changes the display mode and puts it back: run on the PC through `pc run`, since the VM's adapter offers one size"]
fn a_new_display_mode_is_read_again_and_told() {
    let _ = become_per_monitor_aware();
    let (thread, received) = started();
    let remote = thread.remote();
    let before = remote.screens().unwrap();
    let now = screen_size();
    let now = (u32::try_from(now.0).unwrap(), u32::try_from(now.1).unwrap());
    let other = display_sizes()
        .into_iter()
        .find(|&size| size != now && size.0 >= 800)
        .expect("the adapter offers another size");
    let mode = DisplayMode::set(other.0, other.1).unwrap();
    changed(&received);
    assert_eq!(size(&remote.screens().unwrap()), other);
    drop(mode);
    changed(&received);
    assert_eq!(remote.screens().unwrap(), before);
    thread.quit_and_join(JOIN).unwrap();
}
