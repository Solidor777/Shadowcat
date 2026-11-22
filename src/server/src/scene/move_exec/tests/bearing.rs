//! `bearing_degrees`: the `[0, 360)` heading of a displacement, clockwise on the y-down canvas.

use super::*;

#[test]
fn bearing_degrees_normalizes_to_0_360_range() {
    let o = (10.0, 10.0);
    assert_eq!(bearing_degrees(o, (11.0, 10.0), 7.0), 0.0, "+x");
    assert_eq!(bearing_degrees(o, (10.0, 11.0), 7.0), 90.0, "+y");
    assert_eq!(bearing_degrees(o, (9.0, 10.0), 7.0), 180.0, "-x");
    assert_eq!(
        bearing_degrees(o, (10.0, 9.0), 7.0),
        270.0,
        "-y wraps from -90"
    );
    assert_eq!(bearing_degrees(o, (11.0, 11.0), 7.0), 45.0);
    for (dx, dy) in [(1.0, -1e-300), (-1.0, -1e-300), (3.0, -4.0), (-3.0, 4.0)] {
        let b = bearing_degrees(o, (o.0 + dx, o.1 + dy), 7.0);
        assert!((0.0..360.0).contains(&b), "{b} outside [0, 360)");
    }
}

#[test]
fn bearing_degrees_returns_prior_rotation_on_degenerate_start_equals_stop_input() {
    assert_eq!(bearing_degrees((4.0, 4.0), (4.0, 4.0), 123.5), 123.5);
}

/// `token_facing_radians` inverts `bearing_degrees`: reading a freshly auto-rotated rotation back
/// as a direction recovers the displacement's own heading in every quadrant (both ends share one
/// angle convention).
#[test]
fn token_facing_radians_is_the_inverse_of_bearing_degrees() {
    let o = (10.0, 10.0);
    for (dx, dy) in [
        (1.0, 0.0),
        (0.0, 1.0),
        (-1.0, 0.0),
        (0.0, -1.0),
        (3.0, -4.0),
        (-2.0, -5.0),
    ] {
        let rot = bearing_degrees(o, (o.0 + dx, o.1 + dy), 0.0);
        let rad = token_facing_radians(rot);
        let len = f64::hypot(dx, dy);
        assert!((rad.cos() - dx / len).abs() < 1e-9, "cos for ({dx}, {dy})");
        assert!((rad.sin() - dy / len).abs() < 1e-9, "sin for ({dx}, {dy})");
    }
}
