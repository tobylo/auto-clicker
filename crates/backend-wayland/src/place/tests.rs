use super::*;

/// An output at (`x`, `y`) on the layout whose mode is `physical` and
/// whose scale is `steps` / 120, its logical size as niri gives it.
fn output(x: u32, y: u32, physical: (u32, u32), steps: u64) -> Place {
    let logical = |side: u32| u32::try_from(gives(u64::from(side), steps).unwrap()).unwrap();
    Place {
        x,
        y,
        width: logical(physical.0),
        height: logical(physical.1),
        physical: Some(physical),
    }
}

fn at(x: u32, y: u32) -> Option<Point> {
    Some(Point { x, y })
}

#[test]
fn niris_scale_is_the_one_step_that_gives_back_both_logical_sides() {
    let cases = [
        // 3440 x 1440 at 1, 1.25, 1.5, 7/6, 4/3, 2.
        ((3440, 1440), 120),
        ((3440, 1440), 150),
        ((3440, 1440), 180),
        ((3440, 1440), 140),
        ((3440, 1440), 160),
        ((3840, 2160), 240),
        ((2560, 1600), 150),
        ((1920, 1080), 125),
    ];
    for (physical, steps) in cases {
        let place = output(0, 0, physical, steps);
        let scale = Scale::of(physical.0, place.width, physical.1, place.height);
        assert_eq!(scale, Some(Scale::Steps(steps)), "{physical:?} at {steps}");
    }
}

#[test]
fn without_one_fitting_step_the_mode_over_the_logical_size_stands_in() {
    // 10 x 10 logical from 15 x 15: 1.5, 1.4583 and 1.5417 all give it back.
    let scale = Scale::of(15, 10, 15, 10);
    assert_eq!(
        scale,
        Some(Scale::Ratio {
            x: (15, 10),
            y: (15, 10)
        })
    );
    // A logical size no step gives.
    assert!(matches!(
        Scale::of(3440, 2000, 1440, 1440),
        Some(Scale::Ratio { .. })
    ));
    assert_eq!(Scale::of(0, 10, 10, 10), None);
    assert_eq!(Scale::of(10, 10, 10, 0), None);
}

#[test]
fn a_position_maps_to_the_nearest_logical_pixel_on_its_output() {
    // Beside a 2048-wide output, 3440 x 1440 at 1.5: 2293 x 960 logical.
    let right = output(2048, 0, (3440, 1440), 180);
    assert_eq!((right.width, right.height), (2293, 960));
    let cases = [
        ((0, 0), at(2048, 0)),
        ((1, 1), at(2049, 1)),
        ((3, 2), at(2050, 1)),
        ((1720, 720), at(2048 + 1147, 480)),
        // The last physical pixel rounds onto the logical width, which is
        // the neighbour's: kept on its own output.
        ((3439, 1439), at(2048 + 2292, 959)),
    ];
    for ((x, y), expected) in cases {
        assert_eq!(right.point(x, y), expected, "({x}, {y})");
    }
}

#[test]
fn a_position_outside_its_output_or_without_a_mode_is_none() {
    let place = output(0, 0, (2560, 1440), 150);
    for (x, y) in [(-1, 0), (0, -1), (2560, 0), (0, 1440), (i32::MIN, i32::MAX)] {
        assert_eq!(place.point(x, y), None, "({x}, {y})");
    }
    let no_mode = Place {
        physical: None,
        ..place
    };
    assert_eq!(no_mode.point(0, 0), None);
}

#[test]
fn a_turned_output_maps_its_displayed_sides() {
    // 3840 x 2160 turned a quarter at 2: displayed 2160 x 3840, logical
    // 1080 x 1920.
    let turned = output(0, 0, (2160, 3840), 240);
    assert_eq!((turned.width, turned.height), (1080, 1920));
    assert_eq!(turned.point(2159, 3839), at(1079, 1919));
    assert_eq!(turned.point(1001, 2001), at(501, 1001));
}

/// niri's own position for a logical offset `u` at `steps`: rounded.
fn niri(u: f64, steps: u64) -> i32 {
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let scaled = (u * steps as f64 / 120.0).round() as i32;
    scaled
}

#[test]
fn every_position_lands_within_a_logical_pixel_of_the_pointer() {
    for steps in [120, 140, 150, 160, 180, 240] {
        let place = output(0, 0, (3440, 1440), steps);
        let mut u = 0.0;
        while u < f64::from(place.width) {
            let Some(point) = place.point(niri(u, steps), 0) else {
                u += 0.37;
                continue;
            };
            // Half a logical pixel from rounding, half a physical one from
            // niri's.
            #[allow(clippy::cast_precision_loss)]
            let bound = 0.5 + 0.5 * 120.0 / steps as f64 + 1e-9;
            let off = (f64::from(point.x) - u).abs();
            assert!(off <= bound.max(1.0), "{u} at {steps}: {point:?}");
            u += 0.37;
        }
    }
}
