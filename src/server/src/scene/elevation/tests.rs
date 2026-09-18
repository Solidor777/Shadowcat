use super::*;
use crate::data::engine::ElevationBand;

#[test]
fn elevation_or_ground_defaults_and_clamps() {
    assert_eq!(elevation_or_ground(None), GROUND);
    assert_eq!(elevation_or_ground(Some(0.0)), GROUND);
    assert_eq!(elevation_or_ground(Some(3.5)), 3.5);
    assert_eq!(elevation_or_ground(Some(-2.0)), -2.0);
    assert_eq!(elevation_or_ground(Some(f64::NAN)), GROUND);
    assert_eq!(elevation_or_ground(Some(f64::INFINITY)), GROUND);
}

fn band(bottom: Option<f64>, top: Option<f64>) -> ElevationBand {
    ElevationBand { bottom, top }
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

#[test]
fn wall_occludes_non_finite_source_elevation_fails_closed() {
    // Every production caller passes an `elevation_or_ground`-clamped value; the guard keeps
    // the function fail-closed in isolation for any future caller (NaN comparisons would
    // otherwise strip every wall).
    let b = band(Some(0.0), Some(3.0));
    assert!(wall_occludes(Some(&b), f64::NAN));
    assert!(wall_occludes(Some(&b), f64::INFINITY));
    assert!(wall_occludes(Some(&b), f64::NEG_INFINITY));
    assert!(wall_occludes(None, f64::NAN));
}

#[test]
fn band_contains_none_band_contains_everything() {
    assert!(band_contains(None, 0.0));
    assert!(band_contains(None, -1e6));
    assert!(band_contains(None, 1e6));
}

#[test]
fn band_contains_unbounded_top() {
    let b = band(Some(2.0), None);
    assert!(band_contains(Some(&b), 2.0));
    assert!(band_contains(Some(&b), 1e6));
    assert!(!band_contains(Some(&b), 1.0));
}

#[test]
fn band_contains_inverted_interval_fails_closed() {
    let b = band(Some(5.0), Some(1.0));
    assert!(band_contains(Some(&b), 0.0));
    assert!(band_contains(Some(&b), 100.0));
    assert!(band_contains(Some(&b), -100.0));
}

#[test]
fn band_contains_non_finite_endpoint_fails_closed() {
    let nan_end = band(Some(f64::NAN), Some(1.0));
    assert!(band_contains(Some(&nan_end), 0.0));
    let inf_end = band(None, Some(f64::NEG_INFINITY));
    assert!(band_contains(Some(&inf_end), 0.0));
}

#[test]
fn level_of_band_boundaries_and_fallbacks() {
    let levels = vec![
        eng::SceneLevel {
            id: "ground".to_string(),
            name: "Ground".to_string(),
            bottom: 0.0,
            top: 10.0,
            background: None,
        },
        eng::SceneLevel {
            id: "upper".to_string(),
            name: "Upper".to_string(),
            bottom: 10.0,
            top: 20.0,
            background: None,
        },
    ];
    assert_eq!(
        level_of(&levels, 5.0).map(|l| l.id.as_str()),
        Some("ground")
    );
    // A level's top is exclusive: 10.0 belongs to the level starting there.
    assert_eq!(
        level_of(&levels, 10.0).map(|l| l.id.as_str()),
        Some("upper")
    );
    // Above every level: the roof is the top floor.
    assert_eq!(
        level_of(&levels, 99.0).map(|l| l.id.as_str()),
        Some("upper")
    );
    assert!(level_of(&[], 0.0).is_none());
}

mod levels_conformance {
    use super::*;
    use serde::Deserialize;

    /// The shared corpus, read from the client package so both suites see one
    /// file (the formula corpus's shape).
    const CORPUS: &str =
        include_str!("../../../../client/core/src/__fixtures__/levels-conformance.json");

    /// One conformance case: a level list, an elevation, and the level id
    /// `level_of` must resolve to (`null` when no resolution exists).
    #[derive(Deserialize)]
    struct LevelsCase {
        /// Human-readable case label (uniqueness asserted below).
        name: String,
        /// The level list under test.
        levels: Vec<eng::SceneLevel>,
        /// The elevation to resolve.
        elevation: f64,
        /// The expected level id, or `None` for "no level".
        expect: Option<String>,
    }

    /// The corpus envelope.
    #[derive(Deserialize)]
    struct LevelsCorpus {
        /// The case list.
        cases: Vec<LevelsCase>,
    }

    #[test]
    fn level_of_matches_the_shared_corpus() {
        let corpus: LevelsCorpus =
            serde_json::from_str(CORPUS).expect("levels-conformance.json parses");
        let mut seen = std::collections::HashSet::new();
        for case in &corpus.cases {
            assert!(seen.insert(case.name.as_str()), "duplicate case name");
            assert_eq!(
                level_of(&case.levels, case.elevation).map(|l| l.id.clone()),
                case.expect,
                "case '{}'",
                case.name
            );
        }
    }
}
