use super::*;
use crate::data::engine::ElevationBand;

/// A scene (id 10, square grid) plus, per entry in `doors`, one
/// all-flags-authored wall carrying that door state — `None` = a doorless wall.
/// Returns the ECS and the scene id.
fn scene_with_door_walls(doors: &[Option<eng::DoorState>]) -> (SceneEcs, Uuid) {
    let scene_id = crate::test_ids::test_uuid(10);
    let mut scene = crate::data::document::tests::world_scoped_doc(
        crate::test_ids::test_uuid(9),
        scene_id,
        "scene",
    );
    scene.engine = Some(serde_json::json!({
        "grid": { "kind": "square", "size": 100 }, "background": null
    }));
    let mut docs = vec![scene];
    for door in doors {
        let mut wall = crate::data::document::tests::world_scoped_doc(
            crate::test_ids::test_uuid(9),
            Uuid::new_v4(),
            "wall",
        );
        wall.parent_id = Some(scene_id);
        wall.engine = Some(
            serde_json::to_value(eng::WallEngine {
                seg: eng::Seg {
                    x1: 0.0,
                    y1: 0.0,
                    x2: 10.0,
                    y2: 0.0,
                },
                blocks_sight: Some(true),
                blocks_light: Some(true),
                blocks_move: Some(true),
                blocks_sound: None,
                elevation: None,
                door: *door,
                direction: None,
                sight_tier: None,
            })
            .unwrap(),
        );
        docs.push(wall);
    }
    (SceneEcs::from_documents(docs, 0), scene_id)
}

#[test]
fn sight_wall_entries_excludes_an_open_door() {
    // One open door, one closed door, one doorless wall — every one with all
    // authored flags true: the open door drops out, the other two stay.
    let (ecs, scene) = scene_with_door_walls(&[
        Some(eng::DoorState::Open),
        Some(eng::DoorState::Closed),
        None,
    ]);
    assert_eq!(
        ecs.sight_wall_entries(scene).len(),
        2,
        "an open door resolves blocks_sight to false and leaves the sight set"
    );
}

#[test]
fn light_wall_entries_excludes_an_open_door() {
    let (ecs, scene) = scene_with_door_walls(&[
        Some(eng::DoorState::Open),
        Some(eng::DoorState::Locked),
        None,
    ]);
    assert_eq!(
        ecs.light_wall_entries(scene).len(),
        2,
        "an open door resolves blocks_light to false and leaves the light set"
    );
}

/// A scene (id 10) with one all-flags wall carrying the given direction/sight tier.
/// Returns the ECS and the scene id.
fn scene_with_directed_wall(
    direction: Option<f64>,
    sight_tier: Option<eng::WallSightTier>,
) -> (SceneEcs, Uuid) {
    let scene_id = crate::test_ids::test_uuid(10);
    let mut scene = crate::data::document::tests::world_scoped_doc(
        crate::test_ids::test_uuid(9),
        scene_id,
        "scene",
    );
    scene.engine = Some(serde_json::json!({
        "grid": { "kind": "square", "size": 100 }, "background": null
    }));
    let mut wall = crate::data::document::tests::world_scoped_doc(
        crate::test_ids::test_uuid(9),
        Uuid::new_v4(),
        "wall",
    );
    wall.parent_id = Some(scene_id);
    wall.engine = Some(
        serde_json::to_value(eng::WallEngine {
            seg: eng::Seg {
                x1: 0.0,
                y1: 0.0,
                x2: 10.0,
                y2: 0.0,
            },
            blocks_sight: Some(true),
            blocks_light: Some(true),
            blocks_move: Some(true),
            blocks_sound: None,
            elevation: None,
            door: None,
            direction: direction.map(|blocking_side| eng::WallDirection { blocking_side }),
            sight_tier,
        })
        .unwrap(),
    );
    (SceneEcs::from_documents(vec![scene, wall], 0), scene_id)
}

#[test]
fn sight_wall_entries_carries_the_one_way_sign_and_limited_tier() {
    // The sight collector populates BOTH occlusion-metadata fields — the raycast reads
    // them, so they must survive collection.
    let (ecs, scene) = scene_with_directed_wall(Some(1.0), Some(eng::WallSightTier::Limited));
    let entries = ecs.sight_wall_entries(scene);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].seg.one_way, Some(1.0));
    assert!(entries[0].seg.limited);
}

#[test]
fn move_wall_entries_carry_the_one_way_sign_but_never_the_limited_tier() {
    // Movement honors a one-way wall (`Seg::transparent_from` per step) but the partial-
    // blocking tier is sight-only: `limited` must NOT leak into the routing set.
    let (ecs, scene) = scene_with_directed_wall(Some(-1.0), Some(eng::WallSightTier::Limited));
    let entries = ecs.move_wall_entries(scene, None);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].seg.one_way, Some(-1.0));
    assert!(
        !entries[0].seg.limited,
        "a Limited wall still fully blocks movement when blocksMove is set"
    );
}

#[test]
fn light_wall_entries_stay_symmetric_under_a_one_way_or_limited_wall() {
    // Light occlusion is symmetric and immediate by design: neither field propagates.
    let (ecs, scene) = scene_with_directed_wall(Some(1.0), Some(eng::WallSightTier::Limited));
    let entries = ecs.light_wall_entries(scene);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].seg.one_way, None);
    assert!(!entries[0].seg.limited);
}

/// A scene (id 10) with one wall carrying the given authored sight/sound flags and door
/// state. Returns the ECS and the scene id.
fn scene_with_sound_wall(
    blocks_sight: Option<bool>,
    blocks_sound: Option<bool>,
    door: Option<eng::DoorState>,
) -> (SceneEcs, Uuid) {
    let scene_id = crate::test_ids::test_uuid(10);
    let mut scene = crate::data::document::tests::world_scoped_doc(
        crate::test_ids::test_uuid(9),
        scene_id,
        "scene",
    );
    scene.engine = Some(serde_json::json!({
        "grid": { "kind": "square", "size": 100 }, "background": null
    }));
    let mut wall = crate::data::document::tests::world_scoped_doc(
        crate::test_ids::test_uuid(9),
        Uuid::new_v4(),
        "wall",
    );
    wall.parent_id = Some(scene_id);
    wall.engine = Some(
        serde_json::to_value(eng::WallEngine {
            seg: eng::Seg {
                x1: 0.0,
                y1: 0.0,
                x2: 10.0,
                y2: 0.0,
            },
            blocks_sight,
            blocks_light: None,
            blocks_move: None,
            blocks_sound,
            elevation: None,
            door,
            direction: None,
            sight_tier: None,
        })
        .unwrap(),
    );
    (SceneEcs::from_documents(vec![scene, wall], 0), scene_id)
}

#[test]
fn sound_wall_entries_inherits_blocks_sight_when_blocks_sound_absent() {
    // A wall that never authors `blocks_sound` occludes sound exactly as it occludes
    // sight — the default that keeps every stored wall's sound behavior unchanged.
    let (ecs, scene) = scene_with_sound_wall(Some(true), None, None);
    assert_eq!(
        ecs.sound_wall_entries(scene).len(),
        1,
        "absent blocks_sound inherits blocks_sight: true"
    );
    let (ecs, scene) = scene_with_sound_wall(Some(false), None, None);
    assert!(
        ecs.sound_wall_entries(scene).is_empty(),
        "absent blocks_sound inherits blocks_sight: false"
    );
}

#[test]
fn sound_wall_entries_honors_explicit_blocks_sound_true_when_blocks_sight_false() {
    // A window: sound-blocking while sight-transparent — the explicit flag wins.
    let (ecs, scene) = scene_with_sound_wall(Some(false), Some(true), None);
    assert_eq!(ecs.sound_wall_entries(scene).len(), 1);
}

#[test]
fn sound_wall_entries_honors_explicit_blocks_sound_false_when_blocks_sight_true() {
    // A heavy curtain: sight-blocking while sound-transparent — the explicit flag wins.
    let (ecs, scene) = scene_with_sound_wall(Some(true), Some(false), None);
    assert!(ecs.sound_wall_entries(scene).is_empty());
}

#[test]
fn sound_wall_entries_excludes_an_open_door() {
    // An open door is a hole for sound even with an explicit `blocks_sound: true`;
    // a closed door keeps the authored (or inherited) flag.
    let (ecs, scene) = scene_with_sound_wall(Some(true), Some(true), Some(eng::DoorState::Open));
    assert!(
        ecs.sound_wall_entries(scene).is_empty(),
        "the resolver resolves every flag false for an open door"
    );
    let (ecs, scene) = scene_with_sound_wall(Some(true), None, Some(eng::DoorState::Closed));
    assert_eq!(ecs.sound_wall_entries(scene).len(), 1);
}

#[test]
fn sound_wall_entries_stay_symmetric_under_a_one_way_or_limited_wall() {
    // Sound occlusion is symmetric and immediate by design: neither sight-only field
    // propagates into the sound set.
    let (ecs, scene) = scene_with_directed_wall(Some(1.0), Some(eng::WallSightTier::Limited));
    let entries = ecs.sound_wall_entries(scene);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].seg.one_way, None);
    assert!(!entries[0].seg.limited);
}

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

// --- band_overlaps_level: the band-versus-level-span membership rule ---

/// Two-level scene fixture: `ground` [0, 10) and `upper` [10, 20).
fn two_levels() -> Vec<eng::SceneLevel> {
    vec![
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
    ]
}

#[test]
fn band_overlaps_level_span_membership() {
    let levels = two_levels();
    // A band fully inside a level's span applies to that level only.
    assert!(band_overlaps_level(
        Some(&band(Some(2.0), Some(8.0))),
        &levels,
        "ground"
    ));
    assert!(!band_overlaps_level(
        Some(&band(Some(2.0), Some(8.0))),
        &levels,
        "upper"
    ));
    // A band straddling the level boundary applies to BOTH levels (a darkness globe hovering
    // over the boundary shades both floors' fields).
    assert!(band_overlaps_level(
        Some(&band(Some(5.0), Some(12.0))),
        &levels,
        "ground"
    ));
    assert!(band_overlaps_level(
        Some(&band(Some(5.0), Some(12.0))),
        &levels,
        "upper"
    ));
    // Boundary conventions: the band's ends are inclusive, the level's top exclusive — a band
    // whose top lands exactly on a level's bottom shares that one elevation with it, while a
    // band whose bottom lands exactly on a level's top shares nothing.
    assert!(band_overlaps_level(
        Some(&band(Some(0.0), Some(10.0))),
        &levels,
        "upper"
    ));
    assert!(!band_overlaps_level(
        Some(&band(Some(10.0), Some(20.0))),
        &levels,
        "ground"
    ));
    // An absent band end is unbounded; an absent band applies to every level.
    assert!(band_overlaps_level(None, &levels, "ground"));
    assert!(band_overlaps_level(
        Some(&band(None, Some(5.0))),
        &levels,
        "ground"
    ));
    assert!(band_overlaps_level(
        Some(&band(Some(15.0), None)),
        &levels,
        "upper"
    ));
}

#[test]
fn band_overlaps_level_fails_closed_and_grounds_a_level_less_scene() {
    let levels = two_levels();
    // A malformed interval or a non-finite endpoint applies everywhere (the same direction
    // `band_contains` takes).
    assert!(band_overlaps_level(
        Some(&band(Some(10.0), Some(0.0))),
        &levels,
        "ground"
    ));
    assert!(band_overlaps_level(
        Some(&band(Some(f64::NAN), Some(1.0))),
        &levels,
        "upper"
    ));
    // A level-less scene (or an undeclared level id) falls back to the grounded point test —
    // the same answer a mover at ground reads through `band_contains`.
    assert!(band_overlaps_level(
        Some(&band(Some(-1.0), Some(1.0))),
        &[],
        ""
    ));
    assert!(!band_overlaps_level(
        Some(&band(Some(5.0), Some(9.0))),
        &[],
        ""
    ));
    assert!(!band_overlaps_level(
        Some(&band(Some(5.0), Some(9.0))),
        &levels,
        "attic"
    ));
}
