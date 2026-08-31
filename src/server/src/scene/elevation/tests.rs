use super::*;
use crate::data::engine::WallElevation;

#[test]
fn elevation_or_ground_defaults_and_clamps() {
    assert_eq!(elevation_or_ground(None), GROUND);
    assert_eq!(elevation_or_ground(Some(0.0)), GROUND);
    assert_eq!(elevation_or_ground(Some(3.5)), 3.5);
    assert_eq!(elevation_or_ground(Some(-2.0)), -2.0);
    assert_eq!(elevation_or_ground(Some(f64::NAN)), GROUND);
    assert_eq!(elevation_or_ground(Some(f64::INFINITY)), GROUND);
}

fn band(bottom: Option<f64>, top: Option<f64>) -> WallElevation {
    WallElevation { bottom, top }
}

#[test]
fn wall_occludes_absent_band_blocks_everything() {
    assert!(wall_occludes(None, 0.0));
    assert!(wall_occludes(None, 100.0));
    assert!(wall_occludes(None, -100.0));
}

#[test]
fn wall_occludes_band_membership_is_inclusive() {
    let b = band(Some(0.0), Some(3.0));
    assert!(wall_occludes(Some(&b), 0.0));
    assert!(wall_occludes(Some(&b), 3.0));
    assert!(wall_occludes(Some(&b), 1.5));
    // See-over: a source above the top is not occluded.
    assert!(!wall_occludes(Some(&b), 3.5));
    // See-under: a source below the bottom is not occluded.
    assert!(!wall_occludes(Some(&b), -0.5));
}

#[test]
fn wall_occludes_absent_end_is_unbounded() {
    let up = band(Some(2.0), None);
    assert!(wall_occludes(Some(&up), 2.0));
    assert!(wall_occludes(Some(&up), 1e6));
    assert!(!wall_occludes(Some(&up), 1.0));
    let down = band(None, Some(2.0));
    assert!(wall_occludes(Some(&down), 2.0));
    assert!(wall_occludes(Some(&down), -1e6));
    assert!(!wall_occludes(Some(&down), 3.0));
}

#[test]
fn wall_occludes_malformed_interval_fails_closed() {
    let inverted = band(Some(5.0), Some(1.0));
    assert!(wall_occludes(Some(&inverted), 0.0));
    assert!(wall_occludes(Some(&inverted), 100.0));
    assert!(wall_occludes(Some(&inverted), -100.0));
    let nan_end = band(Some(f64::NAN), Some(1.0));
    assert!(wall_occludes(Some(&nan_end), 0.0));
    let inf_end = band(None, Some(f64::NEG_INFINITY));
    assert!(wall_occludes(Some(&inf_end), 0.0));
}
