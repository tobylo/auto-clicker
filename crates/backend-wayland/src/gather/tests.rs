use super::*;

const A: u32 = 7;
const B: u32 = 9;
/// `wl_output.done` with xdg-output 3.
const DONE: Heard = Heard::OutputDone { with_xdg: true };

fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn layout(outputs: &[(Rect, u32)]) -> Layout {
    Layout::new(
        outputs
            .iter()
            .map(|&(rect, scale)| Output::new(rect, scale).unwrap())
            .collect(),
    )
    .unwrap()
}

/// An output at (`x`, `y`) of `width` x `height` logical, at scale 1.
fn screen(gather: &mut Gather, name: u32, (x, y): (i32, i32), (width, height): (i32, i32)) {
    gather.heard(name, Heard::Position { x, y });
    gather.heard(name, Heard::Size { width, height });
    gather.heard(name, Heard::Mode { width, height });
    gather.heard(name, DONE);
}

/// Outputs `A` and `B` at `[x, y, width, height]` each, bound and heard
/// in full.
fn pair([a, b]: [[i32; 4]; 2]) -> Gather {
    let mut gather = Gather::default();
    for (name, [x, y, width, height]) in [(A, a), (B, b)] {
        gather.added(name);
        screen(&mut gather, name, (x, y), (width, height));
    }
    gather
}

/// Two outputs side by side.
fn two() -> Gather {
    pair([[0, 0, 2048, 1280], [2048, 0, 3440, 1440]])
}

/// The layout a whole round trip gives now.
fn settled(gather: &mut Gather) -> Option<Layout> {
    assert!(gather.wants_sync());
    gather
        .synced()
        .expect("nothing was heard during the round trip")
}

#[test]
fn a_layout_is_read_only_after_a_round_trip_with_nothing_new() {
    let mut gather = two();
    assert!(gather.wants_sync());
    assert!(
        !gather.wants_sync(),
        "a second round trip before the first came back"
    );
    let expected = layout(&[
        (rect(0, 0, 2048, 1280), 1000),
        (rect(2048, 0, 3440, 1440), 1000),
    ]);
    assert_eq!(gather.synced(), Some(Some(expected)));
    assert!(!gather.wants_sync(), "nothing heard since");
}

#[test]
fn an_update_to_two_outputs_interleaved_is_never_read_half_done() {
    let mut gather = two();
    settled(&mut gather);
    // A's half of a change arrives, the round trip is asked for, and B's
    // half arrives before it comes back.
    gather.heard(
        A,
        Heard::Size {
            width: 1707,
            height: 1067,
        },
    );
    gather.heard(A, DONE);
    assert!(gather.wants_sync());
    gather.heard(B, Heard::Position { x: 1707, y: 0 });
    assert_eq!(gather.synced(), None, "read with B's half missing");
    gather.heard(B, DONE);
    let expected = layout(&[
        (rect(0, 0, 1707, 1067), 1200),
        (rect(1707, 0, 3440, 1440), 1000),
    ]);
    assert_eq!(settled(&mut gather), Some(expected));
}

#[test]
fn a_round_trip_answered_before_an_outputs_done_reads_nothing() {
    let mut gather = two();
    settled(&mut gather);
    // A's new place, right of B, arrives and the round trip is answered
    // before its size and its done.
    gather.heard(A, Heard::Position { x: 5488, y: 0 });
    assert!(gather.wants_sync());
    assert_eq!(gather.synced(), None, "read A's place with its old size");
    gather.heard(
        A,
        Heard::Size {
            width: 1707,
            height: 1067,
        },
    );
    gather.heard(A, DONE);
    let expected = layout(&[
        (rect(3440, 0, 1707, 1067), 1200),
        (rect(0, 0, 3440, 1440), 1000),
    ]);
    assert_eq!(settled(&mut gather), Some(expected));
}

#[test]
fn with_xdg_output_2_each_object_ends_its_own_update() {
    let mut gather = two();
    settled(&mut gather);
    // A's new place on its xdg-output, then a mode and done on wl_output:
    // the xdg-output's update has not ended.
    gather.heard(A, Heard::Position { x: 5488, y: 0 });
    gather.heard(
        A,
        Heard::Mode {
            width: 2048,
            height: 1280,
        },
    );
    gather.heard(A, Heard::OutputDone { with_xdg: false });
    assert!(gather.wants_sync());
    assert_eq!(gather.synced(), None, "read A's place with its old size");
    gather.heard(
        A,
        Heard::Size {
            width: 1707,
            height: 1067,
        },
    );
    gather.heard(A, Heard::XdgDone);
    let expected = layout(&[
        (rect(3440, 0, 1707, 1067), 1200),
        (rect(0, 0, 3440, 1440), 1000),
    ]);
    assert_eq!(settled(&mut gather), Some(expected));
    // And an xdg-output's done leaves wl_output's update open.
    gather.heard(A, Heard::Scale(2));
    gather.heard(A, Heard::XdgDone);
    assert!(gather.wants_sync());
    assert_eq!(gather.synced(), None, "read before wl_output's done");
}

#[test]
fn an_output_not_heard_in_full_leaves_the_layout_unknown() {
    let mut gather = two();
    gather.added(11);
    gather.heard(11, Heard::Position { x: 0, y: 1440 });
    assert!(gather.wants_sync());
    assert_eq!(gather.synced(), None, "read before the output's done");
    gather.heard(11, DONE);
    assert_eq!(settled(&mut gather), None);
    gather.removed(11);
    assert!(settled(&mut gather).is_some(), "the half-heard output went");
}

#[test]
fn outputs_at_negative_positions_are_moved_to_start_at_the_corner() {
    let mut gather = pair([[-500, -300, 1920, 1080], [1420, 100, 2560, 1440]]);
    let expected = layout(&[
        (rect(0, 0, 1920, 1080), 1000),
        (rect(1920, 400, 2560, 1440), 1000),
    ]);
    assert_eq!(settled(&mut gather), Some(expected));
}

#[test]
fn the_widest_spread_of_positions_still_fits_and_one_past_it_does_not() {
    // B is u32::MAX - 1 from A's corner, one pixel wide: ends at u32::MAX.
    let mut gather = pair([[i32::MIN, 0, 1, 1], [i32::MAX - 1, 0, 1, 1]]);
    let expected = layout(&[
        (rect(0, 0, 1, 1), 1000),
        (rect(u32::MAX - 1, 0, 1, 1), 1000),
    ]);
    assert_eq!(settled(&mut gather), Some(expected));
    gather.heard(B, Heard::Position { x: i32::MAX, y: 0 });
    gather.heard(B, DONE);
    assert_eq!(settled(&mut gather), None, "reaches past u32::MAX");
}

#[test]
fn what_makes_no_layout_leaves_it_unknown() {
    let no_size = |size: (i32, i32)| {
        let mut gather = Gather::default();
        gather.added(A);
        screen(&mut gather, A, (0, 0), size);
        settled(&mut gather)
    };
    assert_eq!(no_size((0, 1080)), None);
    assert_eq!(no_size((1920, 0)), None);
    assert_eq!(no_size((-1920, 1080)), None);

    // Nothing bound.
    let mut empty = Gather::default();
    empty.start();
    assert_eq!(settled(&mut empty), None);

    // More than MAX_OUTPUTS.
    let mut many = Gather::default();
    let count = u32::try_from(auto_clicker_core::recording::MAX_OUTPUTS).unwrap() + 1;
    for name in 0..count {
        many.added(name);
        let x = i32::try_from(name).unwrap() * 100;
        screen(&mut many, name, (x, 0), (100, 100));
    }
    assert_eq!(settled(&mut many), None);
    many.removed(0);
    assert!(settled(&mut many).is_some(), "16 outputs are a layout");
}

#[test]
fn mirrored_outputs_on_one_rectangle_are_a_layout() {
    let mut gather = pair([[0, 0, 1920, 1080], [0, 0, 1920, 1080]]);
    let expected = layout(&[
        (rect(0, 0, 1920, 1080), 1000),
        (rect(0, 0, 1920, 1080), 1000),
    ]);
    assert_eq!(settled(&mut gather), Some(expected));
}

#[test]
fn an_update_that_moves_nothing_gives_the_same_layout() {
    let mut gather = two();
    let before = settled(&mut gather);
    // A re-sent done, and a new refresh rate (the same mode size).
    gather.heard(
        A,
        Heard::Mode {
            width: 2048,
            height: 1280,
        },
    );
    gather.heard(A, DONE);
    assert_eq!(settled(&mut gather), before);
}

#[test]
fn the_scale_is_the_mode_over_the_logical_size() {
    let scale = |mode: Option<(i32, i32)>, turned: bool, whole: Option<i32>, width: i32| {
        let mut gather = Gather::default();
        gather.added(A);
        gather.heard(A, Heard::Position { x: 0, y: 0 });
        gather.heard(A, Heard::Size { width, height: 960 });
        if let Some((width, height)) = mode {
            gather.heard(A, Heard::Mode { width, height });
        }
        gather.heard(A, Heard::Turned(turned));
        if let Some(whole) = whole {
            gather.heard(A, Heard::Scale(whole));
        }
        gather.heard(A, DONE);
        settled(&mut gather).map(|layout| layout.outputs()[0].scale_milli())
    };
    let cases = [
        // niri at 1.5 and 1.4 on 3440 physical (decision 0004).
        ((Some((3440, 1440)), false, None, 2293), Some(1500)),
        ((Some((3440, 1440)), false, None, 2457), Some(1400)),
        // Plasma's panel at 1.25.
        ((Some((2560, 1600)), false, Some(2), 2048), Some(1250)),
        // Turned a quarter: the mode's height is across.
        ((Some((1440, 3440)), true, None, 2293), Some(1500)),
        // No mode: the whole scale, or 1.
        ((None, false, Some(2), 1280), Some(2000)),
        ((None, false, None, 1280), Some(1000)),
        // A scale that is no scale: no layout.
        ((None, false, Some(0), 1280), None),
        ((None, false, Some(-1), 1280), None),
        ((Some((0, 1440)), false, None, 1280), None),
    ];
    for ((mode, turned, whole, width), expected) in cases {
        assert_eq!(
            scale(mode, turned, whole, width),
            expected,
            "{mode:?} {turned} {whole:?} {width}"
        );
    }
}

#[test]
fn what_is_heard_about_an_output_not_bound_is_ignored() {
    let mut gather = two();
    settled(&mut gather);
    gather.heard(99, Heard::Position { x: 5, y: 5 });
    assert!(!gather.wants_sync());
    gather.removed(99);
    assert!(!gather.wants_sync());
}
