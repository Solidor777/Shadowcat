use super::*;
use crate::data::document::{WorldCapDefaults, WorldRole};
use crate::scene::pathfinding::MoveTraits;
use crate::scene::MovementModel;
use serde_json::json;

// --- Fixture helpers (mirrors `scene::mod`'s test `doc` helper verbatim) ---

fn doc(id: u128, parent: Option<u128>, ty: &str) -> crate::data::document::Document {
    let mut d =
        crate::data::document::tests::world_scoped_doc(Uuid::from_u128(9), Uuid::from_u128(id), ty);
    d.parent_id = parent.map(Uuid::from_u128);
    d
}

/// Builds a scene-entity fixture with `engine` set to `body` (`system` stays `{}`) — every
/// reader exercised in `move_exec` (`scene_grid_sizes`, `blocks_move`, `region_field`) is a
/// typed `engine` read; a `token` fixture built through this helper carries no position data
/// any of them consume (`execute_move` takes an explicit `path`, never the token doc's
/// position), so re-rooting it here alongside scene/wall/region is harmless.
fn entity_doc(
    id: u128,
    parent: u128,
    ty: &str,
    body: serde_json::Value,
) -> crate::data::document::Document {
    let mut d = doc(id, Some(parent), ty);
    d.engine = Some(body);
    d
}

/// Scene with a token at (0,0), no walls, cell=100.
fn clear_scene() -> (SceneEcs, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
        ],
        0,
    );
    (ecs, scene_id, token_id)
}

/// Visible set covering all (i,j) in [-1,range) × [-1,range). The lower bound starts at -1,
/// not 0: every fixture's token sits at the scene origin (0.0, 0.0), so a footprint disc
/// anchored anywhere along an axis-hugging path can genuinely reach across x=0/y=0 into the
/// i=-1/j=-1 row or column at up to a 0.5-cell radius — real positive-area overlap, not a
/// gap in the fixture's intent, so the visible set must cover it for a "fully visible"
/// scenario to mean what it says.
fn visible_grid(range: i32) -> BTreeSet<(i32, i32)> {
    (-1..range)
        .flat_map(|i| (-1..range).map(move |j| (i, j)))
        .collect()
}

/// Scene with a token at (0,0) and a wall blocking the step (100,0)→(100,100).
/// Wall segment: x1=50,y1=100,x2=150,y2=100 — horizontal wall at y=100
/// crossing any vertical move between y<100 and y>100 in the x=[50,150] band.
fn walled_scene() -> (SceneEcs, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    // Wall segment perpendicular to the (100,0)→(100,100) step: a horizontal
    // line at y=50 that the vertical segment from (100,0) to (100,100) crosses.
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            entity_doc(
                12,
                10,
                "wall",
                json!({
                    "seg": { "x1": 50, "y1": 50, "x2": 150, "y2": 50 },
                    "blocksMove": true
                }),
            ),
        ],
        0,
    );
    (ecs, scene_id, token_id)
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[test]
fn full_clear_path_reaches_goal() {
    let (ecs, scene, token) = clear_scene();
    // Cells (0,0), (1,0), (1,1) — all visible.
    let visible = visible_grid(3);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(out.stop, (100.0, 100.0));
    assert_eq!(out.render_path.len(), 3);
    assert!(!out.truncated);
}

#[test]
fn wall_truncates_at_last_legal_cell() {
    let (ecs, scene, token) = walled_scene();
    // Wall at y=50 blocks (100,0)→(100,100); first step (0,0)→(100,0) is clear.
    let visible = visible_grid(4);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(out.stop, (100.0, 0.0)); // stopped before the wall
    assert!(out.truncated);
    assert_eq!(out.render_path, vec![(0.0, 0.0), (100.0, 0.0)]);
}

#[test]
fn unseen_cell_truncates_under_visible_restriction() {
    let (ecs, scene, token) = clear_scene();
    // (0,0) and (1,0) visible; (1,1) NOT in the set. (0,-1)/(1,-1) are also required: the
    // token starts at the scene origin (0,0), so the footprint disc at the first step's
    // destination (100,0) — exactly on the y=0 grid line — genuinely reaches into the j=-1
    // row (real positive-area overlap at a 0.4-cell radius), which must be visible too for
    // the first step to succeed as this test intends.
    let mut visible: BTreeSet<(i32, i32)> = BTreeSet::new();
    visible.insert((0, 0));
    visible.insert((1, 0));
    visible.insert((0, -1));
    visible.insert((1, -1));
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(out.stop, (100.0, 0.0));
    assert!(out.truncated);
}

/// Pins the mask gate's behavior once its `supercover_cells` call routes through
/// `SquareGrid::line_traversal` instead of the free function directly. A pure diagonal
/// king-step from a lattice corner (`(0,0)->(100,100)`, cell=100) crosses the shared
/// corner exactly, so the mask check must see all 4 cells the corner-crossing branch
/// emits: the two diagonal cells AND both off-diagonal flankers (mirrors
/// `movement::pure_diagonal_through_corner_includes_both_flanking_cells`). Excluding the
/// flanker `(1,0)` from the mask must still truncate the move at the start cell.
#[test]
fn gate_walk_mask_gate_routes_through_grid_shape_not_hardcoded_supercover() {
    let (ecs, scene, token) = clear_scene();
    let mut visible: BTreeSet<(i32, i32)> = BTreeSet::new();
    visible.insert((0, 0));
    visible.insert((0, 1));
    visible.insert((1, 1));
    // (1,0) deliberately excluded — the corner-crossing flanker cell.
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (200.0, 200.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(out.stop, (0.0, 0.0));
    assert!(out.truncated);
}

/// Documents the `Revealed`-mode caller contract: the `visible` argument must be
/// `visible_cells(...) ∪ explored`. When the union includes an otherwise-unseen cell
/// the move proceeds through it; when the union omits the cell the move truncates there.
#[test]
fn revealed_mode_uses_caller_supplied_union_mask() {
    let (ecs, scene, token) = clear_scene();
    // Cell (1,1) is NOT in the raw visible set but IS in the explored union.
    // The caller is responsible for supplying the union; the executor treats it as opaque.
    // (0,-1)/(1,-1) are also required: the token starts at the scene origin, so the
    // footprint disc at the first step's destination (100,0) genuinely reaches into the
    // j=-1 row at a 0.4-cell radius (see `unseen_cell_truncates_under_visible_restriction`).
    // (0,1) is required too: the second step's destination (100,100) is the 4-way corner
    // shared by (0,0),(1,0),(0,1),(1,1), all genuinely footprint-overlapped at that radius.
    let mut union_mask: BTreeSet<(i32, i32)> = BTreeSet::new();
    union_mask.insert((0, 0));
    union_mask.insert((1, 0));
    union_mask.insert((1, 1)); // explored cell included by caller in the union
    union_mask.insert((0, -1));
    union_mask.insert((1, -1));
    union_mask.insert((0, 1));

    // With the union mask: all supercover cells are present → reaches goal.
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Revealed,
            visible: &union_mask,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(out.stop, (100.0, 100.0));
    assert!(!out.truncated);

    // Without cell (1,1) in the mask: move truncates at (100,0).
    let mut raw_mask: BTreeSet<(i32, i32)> = BTreeSet::new();
    raw_mask.insert((0, 0));
    raw_mask.insert((1, 0));
    raw_mask.insert((0, -1));
    raw_mask.insert((1, -1));
    // (1,1) absent — caller did NOT union in explored; step (100,0)→(100,100) blocked.
    let out2 = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Revealed,
            visible: &raw_mask,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(out2.stop, (100.0, 0.0));
    assert!(out2.truncated);
}

#[test]
fn unrestricted_ignores_mask_but_not_walls() {
    let (ecs, scene, token) = walled_scene();
    // Empty mask — mask is ignored under Unrestricted, but the wall still stops it.
    let empty: BTreeSet<(i32, i32)> = BTreeSet::new();
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(out.stop, (100.0, 0.0)); // mask ignored, wall still stops it
}

#[test]
fn rejects_path_not_starting_at_token() {
    let (ecs, scene, token) = clear_scene();
    let v: BTreeSet<(i32, i32)> = BTreeSet::new();
    assert!(matches!(
        execute_move(
            &ecs,
            MoveGateInputs {
                scene,
                restriction: MovementRestriction::Unrestricted,
                visible: &v,
                cell: FIXTURE_GRID_SIZE,
                budget: None,
                traits: MoveTraits::default(),
            },
            token,
            &[(500.0, 0.0), (600.0, 0.0)],
            false,
            0.4
        ),
        Err(MoveReject::Degenerate)
    ));
}

#[test]
fn long_jump_is_subdivided_and_gated_not_rejected() {
    // A >1-cell authored jump is subdivided by gate_walk and gated per crossed cell, exactly
    // as if the client had sent the explicit intermediate waypoints. All crossed cells here
    // are visible and wall-clear, so the jump succeeds.
    let (ecs, scene, token) = clear_scene();
    let visible = visible_grid(6);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        // 5 cells in one authored jump
        &[(0.0, 0.0), (500.0, 0.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(out.stop, (500.0, 0.0));
    assert!(!out.truncated);
}

#[test]
fn long_jump_truncates_at_the_fog_boundary_mid_segment() {
    // The subdivided jump crosses into an unseen cell partway through the authored
    // segment — the executor must truncate exactly at the fog boundary (a point that is
    // NOT an authored vertex), not admit the whole jump nor reject it outright.
    let (ecs, scene, token) = clear_scene();
    // Only cells (0,0),(1,0),(2,0) are visible; the 5-cell jump would reach unseen (3,0).
    // (0,-1)/(1,-1)/(2,-1) are also required: the token starts at the scene origin and the
    // path runs along y=0, so every dense sample's footprint disc genuinely reaches into the
    // j=-1 row at a 0.4-cell radius (see `unseen_cell_truncates_under_visible_restriction`).
    let mut visible: BTreeSet<(i32, i32)> = BTreeSet::new();
    visible.insert((0, 0));
    visible.insert((1, 0));
    visible.insert((2, 0));
    visible.insert((0, -1));
    visible.insert((1, -1));
    visible.insert((2, -1));
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (500.0, 0.0)],
        false,
        0.4,
    )
    .unwrap();
    assert!(out.truncated);
    assert_eq!(
        out.stop,
        (200.0, 0.0),
        "truncates entering cell (2,0), before unseen cell (3,0)"
    );
    assert_eq!(out.render_path, vec![(0.0, 0.0), (200.0, 0.0)]);
}

#[test]
fn rejects_path_exceeding_gate_walk_cap() {
    // The `TooLong` DoS bound is arc-length/gate-walk-sample based, not vertex-count based:
    // a single segment whose Chebyshev length would require more than MAX_GATE_WALK_SAMPLES
    // sub-steps fails closed, never truncated.
    let (ecs, scene, token) = clear_scene();
    let v: BTreeSet<(i32, i32)> = BTreeSet::new();
    assert!(matches!(
        execute_move(
            &ecs,
            MoveGateInputs {
                scene,
                restriction: MovementRestriction::Unrestricted,
                visible: &v,
                cell: 1.0,
                budget: None,
                traits: MoveTraits::default(),
            },
            token,
            &[(0.0, 0.0), (1.0e7, 0.0)],
            false,
            0.4
        ),
        Err(MoveReject::TooLong)
    ));
}

#[test]
fn rejects_empty_path() {
    let (ecs, scene, token) = clear_scene();
    let v: BTreeSet<(i32, i32)> = BTreeSet::new();
    assert!(matches!(
        execute_move(
            &ecs,
            MoveGateInputs {
                scene,
                restriction: MovementRestriction::Unrestricted,
                visible: &v,
                cell: FIXTURE_GRID_SIZE,
                budget: None,
                traits: MoveTraits::default(),
            },
            token,
            &[(0.0, 0.0)],
            false,
            0.4
        ),
        Err(MoveReject::EmptyPath)
    ));
}

#[test]
fn rejects_unknown_token() {
    let (ecs, scene, _token) = clear_scene();
    let v: BTreeSet<(i32, i32)> = BTreeSet::new();
    let unknown = Uuid::from_u128(999);
    assert!(matches!(
        execute_move(
            &ecs,
            MoveGateInputs {
                scene,
                restriction: MovementRestriction::Unrestricted,
                visible: &v,
                cell: FIXTURE_GRID_SIZE,
                budget: None,
                traits: MoveTraits::default(),
            },
            unknown,
            &[(0.0, 0.0), (100.0, 0.0)],
            false,
            0.4
        ),
        Err(MoveReject::NotAToken)
    ));
}

#[test]
fn unrestricted_full_path_no_walls() {
    let (ecs, scene, token) = clear_scene();
    let empty: BTreeSet<(i32, i32)> = BTreeSet::new();
    // Unrestricted with empty mask should reach the goal with no walls.
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(out.stop, (100.0, 100.0));
    assert!(!out.truncated);
    assert_eq!(out.render_path.len(), 3);
}

fn region_doc(
    id: u128,
    parent: u128,
    behavior: &str,
    cost: f64,
    rect: (f64, f64, f64, f64),
) -> crate::data::document::Document {
    let (x0, y0, x1, y1) = rect;
    entity_doc(
        id,
        parent,
        "region",
        json!({
            "shape": { "kind": "rect", "points": [x0, y0, x1, y1] },
            "behavior": behavior,
            "cost": cost,
            "enabled": true,
        }),
    )
}

#[test]
fn impassable_region_stops_before_entry_like_a_wall() {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            region_doc(12, 10, "impassable", 1.0, (50.0, 0.0, 150.0, 100.0)),
        ],
        0,
    );
    let visible = visible_grid(3);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(
        out.stop,
        (0.0, 0.0),
        "stops BEFORE entering the impassable cell, like a wall"
    );
    assert!(out.truncated);
}

/// Root-cause fix regression: the mask gate's footprint disc is anchored at the true
/// continuous dense-walk sample (`next`), matching the wall gate's anchor exactly, so the two
/// gates now agree on where an off-center `Continuous`-style step truncates. A token starting
/// at (0,30) — interior in y, boundary-exact in x — moving along an axis-aligned segment whose
/// dense subdivision (`k=3`) lands its FIRST intermediate sample exactly at (100,30): that
/// point sits on the x=100 cell-boundary tie, so a 0.4-cell-radius footprint genuinely
/// overlaps BOTH (0,-1) and (1,-1) — cells off the line of travel, not swept by
/// `line_traversal` — in addition to (0,0)/(1,0). Masking out (0,-1)/(1,-1) alone (no wall, no
/// region) must truncate the move exactly at that first dense sample, proving the mask gate's
/// reach now matches the wall gate's `point_segment_distance` reach at the identical anchor.
#[test]
fn continuous_axis_aligned_move_mask_gate_reaches_the_same_boundary_point_the_wall_gate_does() {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 30.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
        ],
        0,
    );
    // (0,-1) and (1,-1) deliberately absent — the flanker cells a boundary-exact footprint
    // disc at (100,30) genuinely overlaps, off the line of travel.
    let mut visible: BTreeSet<(i32, i32)> = BTreeSet::new();
    visible.insert((0, 0));
    visible.insert((1, 0));
    visible.insert((2, 0));
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        // k=3 subdivision (Chebyshev distance 300 over cell 100): dense samples land exactly
        // at x=100,200,300 — none of them an authored vertex.
        &[(0.0, 30.0), (300.0, 30.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(
        out.stop,
        (0.0, 30.0),
        "mask gate truncates at the START, before the first mid-subdivision sample \
         (100,30) — matching where a wall gate anchored at the same point would react, not \
         one dense step further in (the old cell-center-anchored asymmetry)"
    );
    assert!(out.truncated);
}

#[test]
fn arrest_region_stops_at_entry_including_final_step() {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            region_doc(12, 10, "arrest", 1.0, (50.0, -50.0, 150.0, 50.0)),
        ],
        0,
    );
    let visible = visible_grid(3);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        &[(0.0, 0.0), (100.0, 0.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(
        out.stop,
        (100.0, 0.0),
        "arrest stops AT the cell, not before it"
    );
    assert!(
        out.truncated,
        "a final-step arrest reports truncated=true even though the stop is the last vertex"
    );
}

#[test]
fn terrain_region_accumulates_weighted_cost() {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            region_doc(12, 10, "terrain", 2.5, (50.0, 0.0, 150.0, 100.0)),
        ],
        0,
    );
    let visible = visible_grid(3);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        &[(0.0, 0.0), (100.0, 0.0)],
        false,
        0.4,
    )
    .unwrap();
    assert!((out.cost - 2.5).abs() < 1e-9);
}

#[test]
fn impassable_hex_region_stops_a_hex_move_at_the_correct_hex_cell() {
    use crate::scene::grid_shape::{GridShape, HexGrid};
    // Hex scene (size=50): rasterize's candidate enumeration (`GridShape::cells_in_bounds`) and
    // move_exec's region lookup (`grid.cell_of`) must both resolve hex (axial) cells. A rect
    // enclosing ONLY hex cell (1,0)'s center must (a) rasterize onto hex (1,0), and (b) stop a
    // move from (0,0) toward (1,0) before entry. A square-on-hex enumeration or lookup would
    // rasterize onto the wrong axial cell and the move would sail straight through.
    // The scene's declared grid size, the gate's `cell`, and the region rect are all derived
    // from the shape whose `cell_center` supplies this test's coordinates, so none of them can
    // drift apart. The rect is a square of side `size` centred on hex (1,0): half a `size` is
    // under the half-pitch (`√3/2·size ≈ 43.3`) to its axial neighbours on the row and under
    // the row spacing (`1.5·size`) to the four off-row neighbours, so it can only ever contain
    // that one hex's centre — a property the fixture guard asserts rather than assumes.
    let hex = HexGrid { size: 50.0 };
    let c10 = hex.cell_center((1, 0)); // (50·√3, 0) ≈ (86.6, 0)
    let pad = hex.size / 2.0;

    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "hex", "size": hex.size }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 50.0, "h": 50.0, "rotation": 0.0 }),
            ),
            region_doc(
                12,
                10,
                "impassable",
                1.0,
                (c10.0 - pad, c10.1 - pad, c10.0 + pad, c10.1 + pad),
            ),
        ],
        0,
    );

    // (a) The rect rasterizes onto hex cell (1,0) via GridShape, not a square index — and onto
    // NOTHING else. The precondition the truncation assertion depends on is that the blocked
    // set is exactly {(1,0)}, so the six neighbours are asserted clear rather than the one
    // (0,0) the move happens to start in: a rect that spread to a neighbour would stop the
    // move for a reason the test does not name, and a rect derived from the shape must be free
    // to move without that going unnoticed.
    let field = ecs.region_field(scene_id, None).expect("scene exists");
    assert!(
        field.is_impassable((1, 0)),
        "rect rasterizes onto hex cell (1,0)"
    );
    for (n, _, _) in hex.neighbors_with_cost((1, 0), 0) {
        assert!(
            !field.is_impassable(n),
            "fixture: hex {n:?} neighbours the blocked hex and must stay clear"
        );
    }

    // (b) The move stops before entering hex (1,0). Unrestricted → the vision mask is skipped.
    let visible = BTreeSet::new();
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &visible,
            cell: hex.size,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        &[(0.0, 0.0), c10],
        false,
        0.4,
    )
    .unwrap();
    assert!(out.truncated, "an impassable hex region truncates the move");
    assert_eq!(
        hex.cell_of(out.stop),
        (0, 0),
        "the move never enters impassable hex (1,0); it halts in hex (0,0)"
    );
}

#[test]
fn arrest_hex_region_stops_at_entry_composed_with_a_hex_visibility_mask() {
    use crate::scene::grid_shape::{GridShape, HexGrid};
    // Composes the two hex-indexed gates the executor runs per cell-entry — the `Visible` mask
    // gate and the region gate — on ONE hex scene (size=50), and pins both against the
    // square-indexed math that would be used if either lookup regressed.
    //
    // Geometry: a rect enclosing ONLY hex (2,0)'s center (173.2, 0) marks it `arrest`. The
    // square index of that same point is (3,0), so a square `floor(p/cell)` region lookup finds
    // no arrest there and the move sails through. The mask is the exact hex traversal
    // {(0,0),(1,0),(2,0),(3,0)}; the square supercover of the same segment reaches (5,0), so a
    // square-indexed mask gate would instead truncate early, for the wrong reason.
    // The scene's declared grid size, the square supercover's cell size, the gate's `cell`, and
    // the arrest rect are all derived from the shape whose `cell_center` supplies this test's
    // coordinates, so none of them can drift apart. The rect is a square of side `size` centred
    // on hex (2,0): half a `size` is under the half-pitch (`√3/2·size ≈ 43.3`) to its axial
    // neighbours on the row and under the row spacing (`1.5·size`) to the four off-row ones, so
    // it can only ever contain that one hex's centre — asserted, not assumed.
    let hex = HexGrid { size: 50.0 };
    let c20 = hex.cell_center((2, 0)); // (100·√3, 0) ≈ (173.2, 0)
    let c30 = hex.cell_center((3, 0)); // (150·√3, 0) ≈ (259.8, 0)
    let pad = hex.size / 2.0;

    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "hex", "size": hex.size }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 50.0, "h": 50.0, "rotation": 0.0 }),
            ),
            region_doc(
                12,
                10,
                "arrest",
                1.0,
                (c20.0 - pad, c20.1 - pad, c20.0 + pad, c20.1 + pad),
            ),
        ],
        0,
    );

    // Fixture guard, the precondition every assertion in this test rests on: exactly hex
    // (2,0) arrests. Its six
    // neighbours include axial (3,0), which is ALSO the square index of the rect's own
    // location, so the same loop pins the square-indexing claim the test is named for — a
    // square `floor(p/cell)` region lookup would consult (3,0) and find nothing.
    let field = ecs.region_field(scene_id, None).expect("scene exists");
    assert!(
        field.is_arrest((2, 0)),
        "rect rasterizes onto hex cell (2,0)"
    );
    for (n, _, _) in hex.neighbors_with_cost((2, 0), 0) {
        assert!(
            !field.is_arrest(n),
            "fixture: hex {n:?} neighbours the arrest hex and must stay clear"
        );
    }

    let visible: BTreeSet<(i32, i32)> = [(0, 0), (1, 0), (2, 0), (3, 0)].into_iter().collect();
    let square_cells = crate::scene::movement::supercover_cells((0.0, 0.0), c30, hex.size)
        .expect("bounded square supercover");
    assert!(
        square_cells.iter().any(|c| !visible.contains(c)),
        "a square-indexed mask gate would truncate this move for the wrong reason: {square_cells:?}"
    );

    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: hex.size,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        &[(0.0, 0.0), c30],
        false,
        0.4,
    )
    .unwrap();
    assert!(out.truncated, "the arrest hex cell truncates the move");
    assert_eq!(
        hex.cell_of(out.stop),
        (2, 0),
        "arrest stops AT entry into hex (2,0), never before it and never past it"
    );
    // Two cell entries accrue (hex (1,0) then the arrest cell (2,0)); no terrain weighting.
    assert!(
        (out.cost - 2.0).abs() < 1e-9,
        "cost {} accrues per hex cell entry",
        out.cost
    );
}

#[test]
fn authoritative_field_springs_a_secret_region_a_player_was_routed_through() {
    // A gm_only impassable region: move_exec must still enforce it (it always uses the
    // authoritative field), even though a player's pathfind field never saw it.
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let mut secret = region_doc(12, 10, "impassable", 1.0, (50.0, 0.0, 150.0, 100.0));
    secret
        .permissions
        .property_overrides
        .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            secret,
        ],
        0,
    );
    let visible = visible_grid(3);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(
        out.stop,
        (0.0, 0.0),
        "authoritative field springs the secret impassable region"
    );
}

// -----------------------------------------------------------------------
// GM gate-exemption tests: a GM bypasses every gameplay gate (walls, mask,
// impassable, arrest) but no resource guard, and terrain cost still accrues.
// -----------------------------------------------------------------------

/// Empty vision mask — irrelevant to every GM gate-exemption test, since `Unrestricted`
/// skips it.
fn empty_mask() -> BTreeSet<(i32, i32)> {
    BTreeSet::new()
}

/// Token committed at (50,50); a `blocksMove` wall crosses the path's first step
/// (50,50)->(150,50) at x=100, spanning y∈[0,100].
fn scene_with_wall_across_the_path() -> (SceneEcs, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            entity_doc(
                12,
                10,
                "wall",
                json!({
                    "seg": { "x1": 100, "y1": 0, "x2": 100, "y2": 100 },
                    "blocksMove": true
                }),
            ),
        ],
        0,
    );
    (ecs, scene_id, token_id)
}

/// Token committed at (50,50); an impassable region covers [100,200)x[0,100) and an
/// arrest region covers [200,300)x[0,100), so a straight path through both crosses the
/// impassable cell first, then the arrest cell.
fn scene_with_impassable_then_arrest_region() -> (SceneEcs, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            region_doc(12, 10, "impassable", 1.0, (100.0, 0.0, 200.0, 100.0)),
            region_doc(13, 10, "arrest", 1.0, (200.0, 0.0, 300.0, 100.0)),
        ],
        0,
    );
    (ecs, scene_id, token_id)
}

/// Token committed at (50,50); a terrain region with multiplier 3 covers the cell entered
/// by the step (50,50)->(150,50).
fn scene_with_terrain_multiplier_3() -> (SceneEcs, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            region_doc(12, 10, "terrain", 3.0, (100.0, 0.0, 200.0, 100.0)),
        ],
        0,
    );
    (ecs, scene_id, token_id)
}

#[test]
fn gm_move_crosses_a_blocks_move_wall_untruncated() {
    let (ecs, scene, token) = scene_with_wall_across_the_path();
    let path = [(50.0, 50.0), (150.0, 50.0), (250.0, 50.0)];
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &path,
        true,
        0.4,
    )
    .expect("a GM move is admissible");
    assert!(!out.truncated, "a GM move is not truncated by a wall");
    assert_eq!(
        out.render_path.last().copied(),
        Some((250.0, 50.0)),
        "the GM lands at the requested destination"
    );
}

#[test]
fn gm_move_ignores_impassable_and_arrest_regions() {
    let (ecs, scene, token) = scene_with_impassable_then_arrest_region();
    let path = [(50.0, 50.0), (150.0, 50.0), (250.0, 50.0)];
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &path,
        true,
        0.4,
    )
    .expect("admissible");
    assert!(!out.truncated, "neither impassable nor arrest stops a GM");
}

#[test]
fn non_gm_move_is_blocked_by_the_wall_a_gm_crosses() {
    // The exemption is scoped to GMs: the same fixture wall a GM walks through truncates a
    // non-GM.
    let (ecs, scene, token) = scene_with_wall_across_the_path();
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(50.0, 50.0), (150.0, 50.0)],
        false,
        0.4,
    )
    .expect("admissible");
    assert!(out.truncated, "a non-GM is stopped by the wall");
}

#[test]
fn gm_move_is_refused_beyond_the_coordinate_bound() {
    // A GM bypasses gameplay gates but NO resource guard.
    let (ecs, scene, token) = scene_with_wall_across_the_path();
    let over = MAX_GATE_WALK_COORD + 1.0;
    let err = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(50.0, 50.0), (over, 50.0)],
        true,
        0.4,
    )
    .expect_err("a resource guard is never exempted");
    assert!(matches!(err, MoveReject::TooLong), "got {err:?}");
}

#[test]
fn gm_move_accrues_terrain_cost() {
    // Cost is information, not a gate — accrual is independent of the exemption.
    let (ecs, scene, token) = scene_with_terrain_multiplier_3();
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(50.0, 50.0), (150.0, 50.0)],
        true,
        0.4,
    )
    .expect("admissible");
    assert!(
        out.cost >= 3.0,
        "terrain accrues for a GM, got {}",
        out.cost
    );
}

// -----------------------------------------------------------------------
// Continuous (any-angle, non-king-step) unit tests
//
// These paths are not king-step, so the frozen oracle rejects them by shape and no
// differential comparison is possible — pure behavioral tests against `execute_move` only.
// -----------------------------------------------------------------------

#[test]
fn continuous_any_angle_path_reaches_goal_when_fully_visible() {
    let (ecs, scene, token) = clear_scene();
    let visible = visible_grid(4);
    // Any-angle single segment, not axis-aligned, not diagonal-45.
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (350.0, 120.0)],
        false,
        0.4,
    )
    .unwrap();
    assert_eq!(out.stop, (350.0, 120.0));
    assert!(!out.truncated);
    assert_eq!(out.render_path, vec![(0.0, 0.0), (350.0, 120.0)]);
}

#[test]
fn continuous_path_truncates_at_a_wall_crossed_mid_segment() {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    // Vertical wall at x=250, spanning y in [-50,50] — crosses a horizontal move at y=0.
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            entity_doc(
                12,
                10,
                "wall",
                json!({
                    "seg": { "x1": 250, "y1": -50, "x2": 250, "y2": 50 },
                    "blocksMove": true
                }),
            ),
        ],
        0,
    );
    let visible = visible_grid(5);
    // Single authored segment far longer than 1 cell — subdivided by gate_walk into 4
    // dense substeps of 100 units each; the wall sits inside the third substep.
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        &[(0.0, 0.0), (400.0, 0.0)],
        false,
        0.4,
    )
    .unwrap();
    assert!(
        out.truncated,
        "must stop before crossing the wall mid-segment"
    );
    assert_eq!(
        out.stop,
        (200.0, 0.0),
        "stops at the last dense sample before the wall crossing"
    );
}

#[test]
fn continuous_path_stops_before_entering_an_impassable_region_mid_segment() {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            region_doc(12, 10, "impassable", 1.0, (300.0, -50.0, 500.0, 150.0)),
        ],
        0,
    );
    let visible = visible_grid(5);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        &[(0.0, 0.0), (400.0, 0.0)],
        false,
        0.4,
    )
    .unwrap();
    assert!(out.truncated);
    assert_eq!(
        out.stop,
        (200.0, 0.0),
        "stops BEFORE entering impassable cell (3,0) [x=300..400)"
    );
}

#[test]
fn continuous_path_arrest_stops_at_entry_mid_segment_not_before() {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            region_doc(12, 10, "arrest", 1.0, (300.0, -50.0, 500.0, 150.0)),
        ],
        0,
    );
    let visible = visible_grid(5);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        &[(0.0, 0.0), (400.0, 0.0)],
        false,
        0.4,
    )
    .unwrap();
    assert!(out.truncated);
    assert_eq!(
        out.stop,
        (300.0, 0.0),
        "arrest stops AT entry into cell (3,0), not before it"
    );
}

#[test]
fn execute_move_handles_an_any_angle_weighted_continuous_polyline() {
    // A continuous (any-angle) route whose vertices are > 1 cell apart, crossing a terrain
    // cell (mult 3) and stopping at an arrest cell. Proves gate_walk gates + accrues
    // terrain cost + arrests on a continuous polyline with no executor change.
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            region_doc(12, 10, "terrain", 3.0, (100.0, 0.0, 200.0, 100.0)),
            region_doc(13, 10, "arrest", 1.0, (300.0, 0.0, 400.0, 100.0)),
        ],
        0,
    );
    let visible = visible_grid(6);
    // Any-angle polyline: (50,50) -> (250,50) -> (350,50); the first leg is 2 cells in one hop.
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        &[(50.0, 50.0), (250.0, 50.0), (350.0, 50.0)],
        false,
        0.4,
    )
    .expect("executor handles the any-angle weighted polyline");
    assert!(out.truncated, "arrest cell (3,0) truncates the move");
    // Terrain cell (1,0) mult 3 was entered once before the arrest; cost reflects the multiplier.
    assert!(
        out.cost >= 3.0,
        "terrain multiplier accrued, got {}",
        out.cost
    );
}

// -----------------------------------------------------------------------
// Frozen-fixture parity suite for king-step grid inputs
// -----------------------------------------------------------------------

/// Builds a scene with an optional wall and/or region for differential-test scenarios.
/// `wall`: `Some((x1,y1,x2,y2))` adds a `blocksMove` wall segment.
/// `region`: `Some((behavior, cost, x0,y0,x1,y1))` adds a rect region.
/// `secret_region`: when true and `region` is `Some`, marks the region `gm_only`.
fn scene_with_wall_and_region(
    wall: Option<(f64, f64, f64, f64)>,
    region: Option<(&str, f64, f64, f64, f64, f64)>,
    secret_region: bool,
) -> (SceneEcs, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let mut docs = vec![
        entity_doc(
            10,
            0,
            "scene",
            json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
        ),
        entity_doc(
            11,
            10,
            "token",
            json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        ),
    ];
    if let Some((x1, y1, x2, y2)) = wall {
        docs.push(entity_doc(
            12,
            10,
            "wall",
            json!({
                "seg": { "x1": x1, "y1": y1, "x2": x2, "y2": y2 },
                "blocksMove": true
            }),
        ));
    }
    if let Some((behavior, cost, x0, y0, x1, y1)) = region {
        let mut r = region_doc(13, 10, behavior, cost, (x0, y0, x1, y1));
        if secret_region {
            r.permissions
                .property_overrides
                .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
        }
        docs.push(r);
    }
    (SceneEcs::from_documents(docs, 0), scene_id, token_id)
}

struct ExpectedOutcome {
    stop: (f64, f64),
    render_path: Vec<(f64, f64)>,
    truncated: bool,
    cost: f64,
}

struct FrozenCase {
    label: &'static str,
    wall: Option<(f64, f64, f64, f64)>,
    region: Option<(&'static str, f64, f64, f64, f64, f64)>,
    secret_region: bool,
    visible: BTreeSet<(i32, i32)>,
    restriction: MovementRestriction,
    path: Vec<(f64, f64)>,
    expected: ExpectedOutcome,
}

/// Frozen parity fixtures: 10 grid-input scenarios whose expected outcomes are literal
/// constants, computed by nothing at runtime. This pins `execute_move`'s king-step grid
/// behaviour whole — every stop coordinate, render-path vertex, truncation flag and cost —
/// so any change to that behaviour fails here and requires a deliberate fixture edit rather
/// than a silently re-derived expectation.
#[test]
fn frozen_parity_king_step_grid_outcomes() {
    let cases = vec![
        FrozenCase {
            label: "clear scene, full visible, straight path",
            wall: None,
            region: None,
            secret_region: false,
            visible: visible_grid(3),
            restriction: MovementRestriction::Visible,
            path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            expected: ExpectedOutcome {
                stop: (100.0, 100.0),
                render_path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                truncated: false,
                cost: 2.0,
            },
        },
        FrozenCase {
            label: "clear scene, Unrestricted, empty mask",
            wall: None,
            region: None,
            secret_region: false,
            visible: BTreeSet::new(),
            restriction: MovementRestriction::Unrestricted,
            path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            expected: ExpectedOutcome {
                stop: (100.0, 100.0),
                render_path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                truncated: false,
                cost: 2.0,
            },
        },
        FrozenCase {
            label: "wall blocks second step, Visible",
            wall: Some((50.0, 50.0, 150.0, 50.0)),
            region: None,
            secret_region: false,
            visible: visible_grid(4),
            restriction: MovementRestriction::Visible,
            path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            expected: ExpectedOutcome {
                stop: (100.0, 0.0),
                render_path: vec![(0.0, 0.0), (100.0, 0.0)],
                truncated: true,
                cost: 1.0,
            },
        },
        FrozenCase {
            label: "partial mask truncates at unseen cell, Visible",
            wall: None,
            region: None,
            secret_region: false,
            visible: {
                let mut v = BTreeSet::new();
                v.insert((0, 0));
                v.insert((1, 0));
                // The token starts at the scene origin, so the first step's destination
                // (100,0) genuinely reaches the j=-1 row at a 0.4-cell radius (see
                // `unseen_cell_truncates_under_visible_restriction`).
                v.insert((0, -1));
                v.insert((1, -1));
                v
            },
            restriction: MovementRestriction::Visible,
            path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            expected: ExpectedOutcome {
                stop: (100.0, 0.0),
                render_path: vec![(0.0, 0.0), (100.0, 0.0)],
                truncated: true,
                cost: 1.0,
            },
        },
        FrozenCase {
            label: "full mask allowed under Revealed",
            wall: None,
            region: None,
            secret_region: false,
            visible: visible_grid(3),
            restriction: MovementRestriction::Revealed,
            path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            expected: ExpectedOutcome {
                stop: (100.0, 100.0),
                render_path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
                truncated: false,
                cost: 2.0,
            },
        },
        FrozenCase {
            label: "impassable region stops before entry",
            wall: None,
            region: Some(("impassable", 1.0, 50.0, 0.0, 150.0, 100.0)),
            secret_region: false,
            visible: visible_grid(3),
            restriction: MovementRestriction::Unrestricted,
            path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            expected: ExpectedOutcome {
                stop: (0.0, 0.0),
                render_path: vec![(0.0, 0.0)],
                truncated: true,
                cost: 0.0,
            },
        },
        FrozenCase {
            label: "arrest region stops at entry on final step",
            wall: None,
            region: Some(("arrest", 1.0, 50.0, -50.0, 150.0, 50.0)),
            secret_region: false,
            visible: visible_grid(3),
            restriction: MovementRestriction::Unrestricted,
            path: vec![(0.0, 0.0), (100.0, 0.0)],
            expected: ExpectedOutcome {
                stop: (100.0, 0.0),
                render_path: vec![(0.0, 0.0), (100.0, 0.0)],
                truncated: true,
                cost: 1.0,
            },
        },
        FrozenCase {
            label: "terrain region accrues cost",
            wall: None,
            region: Some(("terrain", 2.5, 50.0, 0.0, 150.0, 100.0)),
            secret_region: false,
            visible: visible_grid(3),
            restriction: MovementRestriction::Unrestricted,
            path: vec![(0.0, 0.0), (100.0, 0.0)],
            expected: ExpectedOutcome {
                stop: (100.0, 0.0),
                render_path: vec![(0.0, 0.0), (100.0, 0.0)],
                truncated: false,
                cost: 2.5,
            },
        },
        FrozenCase {
            label: "authoritative field springs a secret impassable region under Visible",
            wall: None,
            region: Some(("impassable", 1.0, 50.0, 0.0, 150.0, 100.0)),
            secret_region: true,
            visible: visible_grid(3),
            restriction: MovementRestriction::Visible,
            path: vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
            expected: ExpectedOutcome {
                stop: (0.0, 0.0),
                render_path: vec![(0.0, 0.0)],
                truncated: true,
                cost: 0.0,
            },
        },
        FrozenCase {
            // The (200,200)->(300,100) leg has both endpoints exactly on 4-way grid-line
            // intersections. `movement::supercover_cells`'s corner-crossing branch gates
            // the diagonal corner-step on a per-axis remaining-step budget so a tMax tie
            // that merely coincides with an axis already at its target cannot drift
            // the traversal past (ei,ej). The CORRECT outcome here is non-truncated.
            label: "diagonal 3-step king path, full visible",
            wall: None,
            region: None,
            secret_region: false,
            visible: visible_grid(4),
            restriction: MovementRestriction::Visible,
            path: vec![(0.0, 0.0), (100.0, 100.0), (200.0, 200.0), (300.0, 100.0)],
            expected: ExpectedOutcome {
                stop: (300.0, 100.0),
                render_path: vec![(0.0, 0.0), (100.0, 100.0), (200.0, 200.0), (300.0, 100.0)],
                truncated: false,
                cost: 3.0,
            },
        },
    ];

    for case in &cases {
        let (ecs, scene, token) =
            scene_with_wall_and_region(case.wall, case.region, case.secret_region);
        let result = execute_move(
            &ecs,
            MoveGateInputs {
                scene,
                restriction: case.restriction,
                visible: &case.visible,
                cell: FIXTURE_GRID_SIZE,
                budget: None,
                traits: MoveTraits::default(),
            },
            token,
            &case.path,
            false,
            0.4,
        );
        let actual =
            result.unwrap_or_else(|e| panic!("{}: expected Ok, got Err({e:?})", case.label));
        assert_eq!(actual.stop, case.expected.stop, "{}: stop", case.label);
        assert_eq!(
            actual.render_path, case.expected.render_path,
            "{}: render_path",
            case.label
        );
        assert_eq!(
            actual.truncated, case.expected.truncated,
            "{}: truncated",
            case.label
        );
        assert!(
            (actual.cost - case.expected.cost).abs() < 1e-9,
            "{}: cost mismatch ({} vs {})",
            case.label,
            actual.cost,
            case.expected.cost
        );
    }
}

// -----------------------------------------------------------------------
// gate_walk tests
// -----------------------------------------------------------------------

#[test]
fn gate_walk_is_identity_on_orthogonal_grid_steps() {
    let path = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)];
    let walk = gate_walk(&path, 100.0).unwrap();
    let positions: Vec<(f64, f64)> = walk.iter().map(|s| s.pos).collect();
    assert_eq!(positions, path.to_vec());
    let authored: Vec<Option<usize>> = walk.iter().map(|s| s.authored_idx).collect();
    assert_eq!(authored, vec![Some(0), Some(1), Some(2)]);
}

#[test]
fn gate_walk_is_identity_on_diagonal_grid_steps() {
    let path = [(0.0, 0.0), (100.0, 100.0), (200.0, 200.0)];
    let walk = gate_walk(&path, 100.0).unwrap();
    let positions: Vec<(f64, f64)> = walk.iter().map(|s| s.pos).collect();
    assert_eq!(positions, path.to_vec());
}

#[test]
fn gate_walk_subdivides_a_long_axis_aligned_segment_into_at_most_one_cell_steps() {
    // (0,0) -> (400,0) at cell=100: Chebyshev length 400 -> subdivided into 4 unit steps.
    let path = [(0.0, 0.0), (400.0, 0.0)];
    let walk = gate_walk(&path, 100.0).unwrap();
    assert_eq!(walk.first().unwrap().pos, (0.0, 0.0));
    assert_eq!(walk.last().unwrap().pos, (400.0, 0.0));
    for w in walk.windows(2) {
        let cheby = (w[1].pos.0 - w[0].pos.0)
            .abs()
            .max((w[1].pos.1 - w[0].pos.1).abs());
        assert!(
            cheby <= 100.0 + 1e-9,
            "step {:?}->{:?} exceeds 1 cell",
            w[0].pos,
            w[1].pos
        );
    }
    // Only the endpoints are authored; interior samples are not.
    assert_eq!(walk.first().unwrap().authored_idx, Some(0));
    assert_eq!(walk.last().unwrap().authored_idx, Some(1));
    assert!(walk[1..walk.len() - 1]
        .iter()
        .all(|s| s.authored_idx.is_none()));
}

#[test]
fn gate_walk_subdivides_a_long_any_angle_segment() {
    // Continuous, non-axis-aligned: (0,0) -> (250, 90) at cell=100.
    // Chebyshev length = max(250, 90) = 250 -> ceil(250/100) = 3 substeps.
    let path = [(0.0, 0.0), (250.0, 90.0)];
    let walk = gate_walk(&path, 100.0).unwrap();
    assert_eq!(walk.len(), 4); // start + 3 substeps
    assert_eq!(walk.last().unwrap().pos, (250.0, 90.0));
    for w in walk.windows(2) {
        let cheby = (w[1].pos.0 - w[0].pos.0)
            .abs()
            .max((w[1].pos.1 - w[0].pos.1).abs());
        assert!(cheby <= 100.0 + 1e-9);
    }
}

#[test]
fn gate_walk_fails_closed_on_non_finite_coordinate() {
    assert!(gate_walk(&[(0.0, 0.0), (f64::NAN, 0.0)], 100.0).is_none());
    assert!(gate_walk(&[(0.0, 0.0), (f64::INFINITY, 0.0)], 100.0).is_none());
}

#[test]
fn gate_walk_fails_closed_on_degenerate_cell() {
    assert!(gate_walk(&[(0.0, 0.0), (100.0, 0.0)], 0.0).is_none());
    assert!(gate_walk(&[(0.0, 0.0), (100.0, 0.0)], -1.0).is_none());
    assert!(gate_walk(&[(0.0, 0.0), (100.0, 0.0)], f64::NAN).is_none());
}

#[test]
fn gate_walk_fails_closed_when_over_the_sample_cap() {
    // A single segment whose subdivision count alone exceeds the cap.
    let path = [(0.0, 0.0), (1.0e7, 0.0)]; // cell=1.0 -> 10,000,000 substeps
    assert!(gate_walk(&path, 1.0).is_none());
}

#[test]
fn gate_walk_fails_closed_when_a_single_segment_lands_exactly_on_the_sample_cap() {
    // k_f == MAX_GATE_WALK_SAMPLES exactly: the walk still needs 1 (start sample) +
    // MAX_GATE_WALK_SAMPLES total samples, one over the cap. Must fail closed, not
    // silently accept an off-by-one under-count.
    let cell = 1.0;
    let cheby = MAX_GATE_WALK_SAMPLES as f64 * cell;
    let path = [(0.0, 0.0), (cheby, 0.0)];
    assert!(gate_walk(&path, cell).is_none());
}

#[test]
fn gate_walk_fails_closed_on_cumulative_cross_segment_sample_cap() {
    // Each segment is individually well under the per-segment cap, but the summed
    // sample count across segments exceeds MAX_GATE_WALK_SAMPLES. The pre-loop
    // per-segment check alone would miss this; only the loop-internal running-total
    // check (`out.len() >= MAX_GATE_WALK_SAMPLES`) catches it.
    let cell = 1.0;
    let seg_len = (MAX_GATE_WALK_SAMPLES / 2 + 100) as f64 * cell; // under the cap alone
    let path = [
        (0.0, 0.0),
        (seg_len, 0.0),
        (seg_len, seg_len), // second segment pushes the running total over the cap
    ];
    assert!(gate_walk(&path, cell).is_none());
}

#[test]
fn gate_walk_rejects_an_authored_path_longer_than_the_sample_cap_before_allocating() {
    // Every step is a genuine 1-cell identity step (no subdivision at all), but the
    // authored vertex count alone exceeds the cap. Must fail closed rather than
    // pre-allocate `Vec::with_capacity(path.len())` for an arbitrarily large `path`.
    let cell = 100.0;
    let path: Vec<(f64, f64)> = (0..=(MAX_GATE_WALK_SAMPLES + 1))
        .map(|i| (i as f64 * cell, 0.0))
        .collect();
    assert!(gate_walk(&path, cell).is_none());
}

#[test]
fn gate_walk_is_identity_on_non_round_cell_size_under_floating_point_noise() {
    // Non-round cell (a perfectly normal GM-configured value; `Grid.size` carries no
    // round-number constraint). A zero-tolerance `cheby <= cell` comparison
    // spuriously subdivides some fraction of genuine single-cell steps here due to
    // independent floating-point rounding in the two coordinate subtractions.
    let cell = 33.33_f64;
    for i in 0..2000u32 {
        let base = i as f64 * cell;
        // Orthogonal single-cell step.
        let ortho = [(base, 0.0), (base + cell, 0.0)];
        let walk = gate_walk(&ortho, cell).unwrap();
        assert_eq!(
            walk.len(),
            2,
            "orthogonal single-cell step at i={i} was spuriously subdivided: {walk:?}"
        );
        // Diagonal single-cell step.
        let diag = [(base, base), (base + cell, base + cell)];
        let walk = gate_walk(&diag, cell).unwrap();
        assert_eq!(
            walk.len(),
            2,
            "diagonal single-cell step at i={i} was spuriously subdivided: {walk:?}"
        );
    }
}

#[test]
fn gate_walk_fails_closed_on_extreme_magnitude_coordinate_instead_of_false_identity() {
    // At large enough base
    // coordinates the magnitude-scaled tolerance can itself exceed a full cell length,
    // silently collapsing a genuinely-multi-cell segment (cheby == cell + 1.0, which must
    // subdivide into 2 substeps) into a false single-step identity. Reproduced directly at
    // base=1e14, cell=33.33 (tol there is ~2.84, already far
    // past the 1.0 excess this segment carries) — well above `MAX_GATE_WALK_COORD` (1e9), so
    // the bound must reject it outright (fail closed) rather than let the tolerance
    // misclassify it.
    let cell = 33.33_f64;
    let base = 1.0e14_f64;
    let path = [(base, 0.0), (base + cell + 1.0, 0.0)];
    assert!(
        gate_walk(&path, cell).is_none(),
        "extreme-magnitude segment must fail closed, not silently collapse to identity"
    );
}

#[test]
fn gate_walk_fails_closed_on_coordinate_over_the_magnitude_bound() {
    // Direct test of the new bound itself: a coordinate just over `MAX_GATE_WALK_COORD`
    // must be rejected even on an otherwise-trivial single-cell step (isolates the bound
    // check from `gate_walk_fails_closed_when_the_tolerance_would_exceed_a_cell`'s
    // tolerance-overshoot scenario).
    let cell = 100.0_f64;
    let over = MAX_GATE_WALK_COORD + 1.0;
    assert!(gate_walk(&[(over, 0.0), (over + cell, 0.0)], cell).is_none());
    assert!(gate_walk(&[(0.0, over), (0.0, over + cell)], cell).is_none());
}

#[test]
fn gate_walk_accepts_coordinate_at_the_magnitude_bound() {
    // A coordinate exactly AT `MAX_GATE_WALK_COORD` (not over it) must not be rejected by
    // the bound check itself — confirms the comparison is strictly `>`, not `>=`.
    let cell = 100.0_f64;
    let at = MAX_GATE_WALK_COORD;
    let walk = gate_walk(&[(at - cell, 0.0), (at, 0.0)], cell).unwrap();
    assert_eq!(walk.len(), 2);
}

#[test]
fn gate_walk_on_empty_path_returns_empty() {
    let walk = gate_walk(&[], 100.0).unwrap();
    assert!(walk.is_empty());
}

// -----------------------------------------------------------------------
// Hex-scene integration coverage: proves the fully-wired hex path (walls + the
// visibility mask) behaves correctly end-to-end through `execute_move`, mirroring this
// module's square-scene wall/mask tests.
// -----------------------------------------------------------------------

/// The ONE expression of the grid size every 100-unit scene in this module is built on: each
/// such scene declares it as its own `grid.size`, `hex_cell_center` builds its `HexGrid` at
/// it, `scene_with_narrow_gap_and_wide_token` divides its corridor span by it to author
/// bounds, and every test gating one of those scenes passes it as `MoveGateInputs.cell`. A
/// scene configured at one size and gated at another would test a grid no fixture declared,
/// and nothing else here would notice.
///
/// It does NOT cover every scene here.
/// `impassable_hex_region_stops_a_hex_move_at_the_correct_hex_cell` and
/// `arrest_hex_region_stops_at_entry_composed_with_a_hex_visibility_mask` build their own
/// `HexGrid` at a size this constant does not carry, and bind their declared and gated sizes
/// to that shape's own `size` field instead — the same no-restatement rule, expressed against
/// a different shape rather than against this value. `rejects_path_exceeding_gate_walk_cap`
/// deliberately gates a `clear_scene` at `cell: 1.0`, a mismatch that IS the input under
/// test; the `gate_walk` unit tests configure no scene at all.
const FIXTURE_GRID_SIZE: f64 = 100.0;

/// Pointy-top axial hex center, delegating to the shape rather than restating its formula. The
/// scenes reading it declare `FIXTURE_GRID_SIZE` as their own grid size, and
/// `scene_with_narrow_gap_and_wide_token` divides by that same constant to author its bounds,
/// so a second expression of `HexGrid::axial_to_pixel` here would move a token position out
/// from under the grid it is indexed against.
fn hex_cell_center(q: i32, r: i32) -> (f64, f64) {
    crate::scene::grid_shape::GridShape::cell_center(
        &crate::scene::grid_shape::HexGrid {
            size: FIXTURE_GRID_SIZE,
        },
        (q, r),
    )
}

/// Hex scene on `FIXTURE_GRID_SIZE` with a token at axial (0,0), no walls.
fn hex_clear_scene() -> (SceneEcs, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(20);
    let token_id = Uuid::from_u128(21);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                20,
                0,
                "scene",
                json!({ "grid": { "kind": "hex", "size": FIXTURE_GRID_SIZE },
                        "background": null }),
            ),
            entity_doc(
                21,
                20,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
        ],
        0,
    );
    (ecs, scene_id, token_id)
}

/// Hex scene on `FIXTURE_GRID_SIZE` with a vertical `blocksMove` wall at `wall_x`.
fn hex_walled_scene(wall_x: f64) -> (SceneEcs, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(20);
    let token_id = Uuid::from_u128(21);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                20,
                0,
                "scene",
                json!({ "grid": { "kind": "hex", "size": FIXTURE_GRID_SIZE },
                        "background": null }),
            ),
            entity_doc(
                21,
                20,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            entity_doc(
                22,
                20,
                "wall",
                json!({
                    "seg": { "x1": wall_x, "y1": -50.0, "x2": wall_x, "y2": 50.0 },
                    "blocksMove": true
                }),
            ),
        ],
        0,
    );
    (ecs, scene_id, token_id)
}

#[test]
fn hex_scene_gate_walk_blocks_entry_at_a_wall() {
    // Axial path (0,0)->(1,0)->(2,0). A wall crossing exactly the (1,0)->(2,0) center step
    // stops the executor at (1,0), never reaching the goal.
    let c1 = hex_cell_center(1, 0);
    let c2 = hex_cell_center(2, 0);
    let wall_x = (c1.0 + c2.0) / 2.0;
    let (ecs, scene, token) = hex_walled_scene(wall_x);
    let empty: BTreeSet<(i32, i32)> = BTreeSet::new();
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), c1, c2],
        false,
        0.4,
    )
    .unwrap();
    assert!(out.truncated, "must stop before crossing the hex wall");
    assert!(
        (out.stop.0 - c1.0).abs() < 1e-6 && (out.stop.1 - c1.1).abs() < 1e-6,
        "stops at the last legal hex cell before the wall, got {:?}",
        out.stop
    );
}

#[test]
fn hex_scene_gate_walk_respects_the_visibility_mask() {
    // Axial path (0,0)->(1,0)->(2,0) under Visible restriction; the mask covers (0,0) and
    // (1,0) but excludes (2,0) — the executor must stop before entering the excluded cell.
    let (ecs, scene, token) = hex_clear_scene();
    let c1 = hex_cell_center(1, 0);
    let c2 = hex_cell_center(2, 0);
    let mut visible: BTreeSet<(i32, i32)> = BTreeSet::new();
    visible.insert((0, 0));
    visible.insert((1, 0));
    // (2, 0) deliberately excluded.
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), c1, c2],
        false,
        0.4,
    )
    .unwrap();
    assert!(
        out.truncated,
        "must not reach the masked-out hex cell (2,0)"
    );
    assert!(
        out.stop.0 < c2.0 - 1e-6,
        "stop must land before the excluded cell's center, got {:?}",
        out.stop
    );
}

// -----------------------------------------------------------------------
// Footprint-aware gate tests: `execute_move` adopts `cell_enterable`'s footprint
// predicate set instead of gating on the mover's center cell alone.
// -----------------------------------------------------------------------

/// Parentless config/actor doc builder (mirrors `entity_doc` for the parented case).
fn entity_doc_top(id: u128, ty: &str, body: serde_json::Value) -> crate::data::document::Document {
    let mut d = doc(id, None, ty);
    d.engine = Some(body);
    d
}

/// A minimal, structurally-complete `ActorEngine` body with no vision override (falls back
/// to the unlimited-range default), mirroring `scene::mod`'s own `actor_body_shaped` fixture.
fn actor_body_shaped(shape: &str, w: f64, h: f64) -> serde_json::Value {
    json!({
        "displayName": "Fixture Actor",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": w, "h": h },
        "shape": shape,
        "conditions": [],
        "prototype": true,
    })
}

/// Same as `actor_body_shaped`, plus an explicit vision assignment (range in grid cells).
fn actor_body_shaped_with_vision(
    shape: &str,
    w: f64,
    h: f64,
    vision: serde_json::Value,
) -> serde_json::Value {
    json!({
        "displayName": "Fixture Actor",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": w, "h": h },
        "shape": shape,
        "conditions": [],
        "prototype": true,
        "vision": vision,
    })
}

/// World settings for the footprint fixtures: `visible`/losRestriction off/lighting
/// all-bright (globalIllumination) — vision is driven entirely by each token's OWN vision
/// assignment (range) rather than by wall/light geometry, keeping every fixture's mask
/// derivable by hand from `resolve_token_footprint` + `visible_cells` alone.
fn footprint_world_settings(model: &str) -> serde_json::Value {
    json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "globalIllumination",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "visible",
            "movementModel": model,
            "partialCellLeniency": false
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    })
}

/// A corridor split by a single vertical wall, gapped centered on the token's row, between
/// grid-aligned start/goal cells 4 columns apart. The gap is sized PER KIND from the token's
/// OWN resolved footprint radius (`resolve_token_footprint`), since a hex token is wider than
/// its square counterpart at the same authored size (owner ruling: a hex's authored size
/// counts HEXES, so its collision radius is the circumradius `max(w,h)`, not the `max(w,h)/2`
/// this circle-shaped actor resolves to on square) — square: 300-unit gap for a footprint-1.6-cell token
/// (r_scene=80, 70-unit margin); hex: 460-unit gap for the same actor (r_scene=160, matching
/// 70-unit margin). `start`/`goal` are computed
/// from the ACTUAL grid shape's own `cell_center` (square: `(i+0.5)*cell`; hex: the axial
/// pointy-top formula `hex_cell_center` uses) rather than a single literal shared across both
/// kinds — hex cell centers do not fall on square-grid-aligned coordinates, and `execute_move`
/// requires `path[0]` to equal the token's committed position exactly. Returns `(ecs, scene,
/// token, user, start, goal)`. Vision is unlimited (no vision override) so the whole corridor
/// is visible to `user`, isolating the wall/footprint interaction under test.
///
/// The authored bounds are a per-axis dimension measured in grid units (cells), continuous —
/// never world units, and not required to be integral (the hex arm's width is `12.660254`) —
/// so the corridor's world span is divided by `FIXTURE_GRID_SIZE`, the same constant the
/// scene's own grid declares and `hex_cell_center` builds its shape from, read once rather
/// than restated at any of the three. On square that reproduces the corridor rectangle exactly; on hex the block's
/// world rectangle is a shear-dependent function that is strictly LARGER than the corridor
/// span, which preserves the fixture's intent — the play area covers the corridor and the gap
/// with room to spare — a fortiori.
fn scene_with_narrow_gap_and_wide_token(
    kind: &str,
    model: MovementModel,
) -> (SceneEcs, Uuid, Uuid, Uuid, (f64, f64), (f64, f64)) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let user = Uuid::from_u128(1);
    let model_str = match model {
        MovementModel::GridStepped => "grid-stepped",
        MovementModel::Continuous => "continuous",
    };
    let (start, goal) = if kind == "hex" {
        (hex_cell_center(0, 2), hex_cell_center(4, 2))
    } else {
        (
            (0.5 * FIXTURE_GRID_SIZE, 2.5 * FIXTURE_GRID_SIZE),
            (4.5 * FIXTURE_GRID_SIZE, 2.5 * FIXTURE_GRID_SIZE),
        )
    };
    // Both grids place `start`/`goal` on the same row (`y` depends only on the row index, not
    // the column), so a single wall gap centered on that shared `y` clears both kinds.
    let row_y = start.1;
    let wall_x = (start.0 + goal.0) / 2.0;
    // Gap half-height, derived from each kind's OWN resolved footprint radius (a token's
    // authored size counts HEXES on hex, so its collision radius is the circumscribing
    // radius, wider than a square block's half-diagonal at the same authored size): square's
    // r_scene is 80 (`max(w,h)/2` circle formula — 1.6/2 · FIXTURE_GRID_SIZE), so its
    // 150-unit half-gap carries a 70-unit margin; hex's r_scene is 160 (`max(w,h)` ·
    // FIXTURE_GRID_SIZE), so its half-gap is 230 for the same 70-unit margin — a narrower gap
    // would correctly refuse this token.
    let gap_half_height = if kind == "hex" { 230.0 } else { 150.0 };
    let mut tok = entity_doc(
        11,
        10,
        "token",
        json!({ "x": start.0, "y": start.1, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": Uuid::from_u128(200).to_string() }),
    );
    tok.owner = Some(user);
    let wall = |id: u128, x1: f64, y1: f64, x2: f64, y2: f64| {
        entity_doc(
            id,
            10,
            "wall",
            json!({ "seg": {"x1":x1,"y1":y1,"x2":x2,"y2":y2}, "blocksMove": true }),
        )
    };
    let scene = entity_doc(
        10,
        0,
        "scene",
        json!({ "grid": { "kind": kind, "size": FIXTURE_GRID_SIZE }, "background": null,
                "bounds": { "width": (goal.0 + 400.0) / FIXTURE_GRID_SIZE,
                            "height": (row_y + 400.0) / FIXTURE_GRID_SIZE } }),
    );
    let mut ecs = SceneEcs::from_documents(
        vec![
            scene,
            tok,
            wall(12, wall_x, 0.0, wall_x, row_y - gap_half_height),
            wall(13, wall_x, row_y + gap_half_height, wall_x, row_y + 2000.0),
        ],
        0,
    );
    ecs.set_actors(vec![entity_doc_top(
        200,
        "actor",
        actor_body_shaped("circle", 1.6, 1.6),
    )]);
    ecs.set_world_settings_for_test(footprint_world_settings(model_str));
    (ecs, scene_id, token_id, user, start, goal)
}

/// A `blocksMove` wall at x=100, sitting exactly midway (0.5 cell) between cell (0,0)'s center
/// (50,50) and cell (1,0)'s center (150,50) — clears the default 0.4-cell footprint disc test
/// (0.5 > 0.4) but still crosses the direct center-to-center step. The wall's y-span is huge
/// (±2000, far past the search window) so no detour around either end exists: the
/// `gate_refused_steps_are_absent_from_every_route_non_gm_grid` needs column 1 genuinely
/// UNREACHABLE, not merely blocked on the
/// direct step (a
/// finite wall's north/south end would let a detour reach the same destination cell via a
/// different route, which the test cannot distinguish from a gate/router disagreement). The
/// token carries no actor, so `resolve_token_footprint` falls back to the 0.4 default.
fn scene_with_wall_between_adjacent_cells_and_default_footprint() -> (SceneEcs, Uuid, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let user = Uuid::from_u128(1);
    let mut tok = entity_doc(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let wall = entity_doc(
        12,
        10,
        "wall",
        json!({ "seg": {"x1":100.0,"y1":-2000.0,"x2":100.0,"y2":2000.0}, "blocksMove": true }),
    );
    let scene = entity_doc(
        10,
        0,
        "scene",
        json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null,
                "bounds": { "width": 3.0, "height": 3.0 } }),
    );
    let mut ecs = SceneEcs::from_documents(vec![scene, tok, wall], 0);
    ecs.set_world_settings_for_test(footprint_world_settings("grid-stepped"));
    (ecs, scene_id, token_id, user)
}

/// A wide token (circle, size 1.2 cells ⇒ footprint radius 0.6, over the 0.5-cell half-width)
/// whose vision is explicitly range-limited to 1.2 cells: this lights the token's own cell
/// (0,0) and the straight-ahead cell (1,0) (both within range), but excludes the destination
/// cell's PERPENDICULAR neighbors (1,-1)/(1,1) (~1.414 cells away) and the next cell along the
/// row (2,0) (2.0 cells away) — every one of which the 0.6-radius footprint disc at (1,0)
/// overlaps, so each is a candidate the mask must include for the move to succeed.
fn scene_with_lit_center_line_only() -> (SceneEcs, Uuid, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let user = Uuid::from_u128(1);
    let mut tok = entity_doc(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    tok.embedded.insert(
        "actor".into(),
        vec![{
            let mut a = doc(99, None, "actor");
            a.engine = Some(actor_body_shaped_with_vision(
                "circle",
                1.2,
                1.2,
                json!([{ "mode": "normal", "range": 1.2 }]),
            ));
            a
        }],
    );
    let scene = entity_doc(
        10,
        0,
        "scene",
        json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null,
                "bounds": { "width": 3.0, "height": 3.0 } }),
    );
    let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
    ecs.set_world_settings_for_test(footprint_world_settings("grid-stepped"));
    (ecs, scene_id, token_id, user)
}

/// A fully open, fully visible scene (no walls, no vision override ⇒ unlimited range) — the
/// `is_gm`-free counterpart of the wall/region fixtures, for tests that need admissible
/// footprint movement with nothing else in play.
fn scene_with_open_lit_area() -> (SceneEcs, Uuid, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let user = Uuid::from_u128(1);
    let mut tok = entity_doc(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let scene = entity_doc(
        10,
        0,
        "scene",
        json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null,
                "bounds": { "width": 3.0, "height": 3.0 } }),
    );
    let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
    ecs.set_world_settings_for_test(footprint_world_settings("grid-stepped"));
    (ecs, scene_id, token_id, user)
}

/// A wide token (circle, size 1.2 ⇒ footprint radius 0.6) stepping (0,0)->(1,0) with an
/// `arrest` region tightly enclosing NEIGHBOR cell (1,1)'s center only — a cell the 0.6-radius
/// footprint disc at (1,0) overlaps (so it must stay in the mask, hence no vision override:
/// unlimited range) but the mover's CENTER never enters. Arrest must not spring here.
fn scene_with_arrest_cell_beside_the_path_and_wide_token() -> (SceneEcs, Uuid, Uuid, Uuid) {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let user = Uuid::from_u128(1);
    let mut tok = entity_doc(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": Uuid::from_u128(200).to_string() }),
    );
    tok.owner = Some(user);
    let scene = entity_doc(
        10,
        0,
        "scene",
        json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null,
                "bounds": { "width": 3.0, "height": 3.0 } }),
    );
    let mut ecs = SceneEcs::from_documents(
        vec![
            scene,
            tok,
            region_doc(12, 10, "arrest", 1.0, (100.0, 100.0, 200.0, 200.0)),
        ],
        0,
    );
    ecs.set_actors(vec![entity_doc_top(
        200,
        "actor",
        actor_body_shaped("circle", 1.2, 1.2),
    )]);
    ecs.set_world_settings_for_test(footprint_world_settings("grid-stepped"));
    (ecs, scene_id, token_id, user)
}

/// Empty vision mask helper reused by `execute_move_refuses_an_out_of_range_footprint`
/// (`empty_mask`); this scene just needs a wall the Degenerate guard
/// must reject BEFORE any per-step gating runs.
fn scene_with_wall_across_the_path_for_footprint_guard() -> (SceneEcs, Uuid, Uuid) {
    scene_with_wall_across_the_path()
}

#[test]
fn route_admissible_implies_gate_admissible_for_a_non_gm_grid() {
    // Forward-parity direction, GridStepped-scoped: there `gate_walk` is the identity on
    // cell-center input, so the gate's sample points ARE the cell centers `cell_enterable`
    // evaluates at, giving the STRONG route ⇔ gate equivalence. This test only asserts the
    // ⇒ half (the ⇐ half is `gate_refused_steps_are_absent_from_every_route_non_gm_grid`
    // ). Continuous is NOT covered here — `gate_walk`'s dense sampling and the router's
    // cell-center evaluation operate at different granularity there, so only the WEAKER
    // route ⊆ gate-allowed direction holds; see
    // `route_admissible_implies_gate_admissible_for_a_non_gm_continuous`.
    for kind in ["square", "hex"] {
        let (ecs, scene, token, user, start, goal) =
            scene_with_narrow_gap_and_wide_token(kind, MovementModel::GridStepped);
        let fp = ecs.resolve_token_footprint(token, scene).expect("in-range");
        let mask = ecs.visible_cells(
            user,
            WorldRole::Player,
            &WorldCapDefaults::default(),
            scene,
            false,
        );
        // NOT `if let Ok` — a fixture that yields no route must fail the test, not skip it.
        let route = ecs
            .pathfind(
                crate::scene::RouteRequester {
                    user,
                    is_gm: false,
                    world_role: WorldRole::Player,
                    world_defaults: &WorldCapDefaults::default(),
                    explored: None,
                },
                scene,
                start,
                &[goal],
                crate::scene::RouteMover {
                    footprint_radius: fp,
                    budget_cells: None,
                    traits: MoveTraits::default(),
                },
            )
            .expect("the fixture is routable for this footprint");
        let out = execute_move(
            &ecs,
            MoveGateInputs {
                scene,
                restriction: MovementRestriction::Visible,
                visible: &mask,
                cell: FIXTURE_GRID_SIZE,
                budget: None,
                traits: MoveTraits::default(),
            },
            token,
            &route.path,
            false,
            fp,
        )
        .expect("a routed path is admissible");
        assert!(
            !out.truncated,
            "kind={kind}: the gate accepts every routed step"
        );
    }
}

#[test]
fn route_admissible_implies_gate_admissible_for_a_non_gm_continuous() {
    // Weaker route ⊆ gate-allowed direction, Continuous-scoped: `gate_walk`'s dense sampling and the router's
    // cell-center evaluation operate at different granularity on this model, so only route ⊆
    // gate-allowed holds here — NOT the reverse/equivalence, which holds for GridStepped only
    // (see `route_admissible_implies_gate_admissible_for_a_non_gm_grid`). This reuses
    // `scene_with_narrow_gap_and_wide_token`'s existing `Continuous` dispatch arm: no region
    // is present, so `pathfind` takes the pure-polyanya branch and returns a genuine
    // multi-sample any-angle route through the 300-unit gap (footprint radius 80), not a
    // degenerate single-point route — this test's route-length assertion rules out a
    // vacuous pass.
    let (ecs, scene, token, user, start, goal) =
        scene_with_narrow_gap_and_wide_token("square", MovementModel::Continuous);
    let fp = ecs.resolve_token_footprint(token, scene).expect("in-range");
    let mask = ecs.visible_cells(
        user,
        WorldRole::Player,
        &WorldCapDefaults::default(),
        scene,
        false,
    );
    let route = ecs
        .pathfind(
            crate::scene::RouteRequester {
                user,
                is_gm: false,
                world_role: WorldRole::Player,
                world_defaults: &WorldCapDefaults::default(),
                explored: None,
            },
            scene,
            start,
            &[goal],
            crate::scene::RouteMover {
                footprint_radius: fp,
                budget_cells: None,
                traits: MoveTraits::default(),
            },
        )
        .expect("the fixture is routable for this footprint");
    assert!(
        route.path.len() >= 2,
        "the any-angle polyanya route must actually traverse the gap, not collapse to a point"
    );
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &mask,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &route.path,
        false,
        fp,
    )
    .expect("a router-admissible continuous route is gate-admissible");
    assert!(
        !out.truncated,
        "the gate accepts every step of the router's own any-angle route"
    );
}

#[test]
fn gate_refused_steps_are_absent_from_every_route_non_gm_grid() {
    // Reverse-parity direction: catches a gate MORE
    // permissive than the router (e.g. a gate that omits the `segments_cross` check).
    let (ecs, scene, token, user) = scene_with_wall_between_adjacent_cells_and_default_footprint();
    let fp = ecs.resolve_token_footprint(token, scene).expect("in-range"); // 0.4
    let mask = ecs.visible_cells(
        user,
        WorldRole::Player,
        &WorldCapDefaults::default(),
        scene,
        false,
    );
    let candidates = [
        [(50.0, 50.0), (150.0, 50.0)],
        [(50.0, 50.0), (150.0, 150.0)],
        [(50.0, 50.0), (50.0, 150.0)],
    ];
    for path in candidates {
        let out = execute_move(
            &ecs,
            MoveGateInputs {
                scene,
                restriction: MovementRestriction::Visible,
                visible: &mask,
                cell: FIXTURE_GRID_SIZE,
                budget: None,
                traits: MoveTraits::default(),
            },
            token,
            &path,
            false,
            fp,
        )
        .expect("admissible input");
        if out.truncated {
            let route = ecs.pathfind(
                crate::scene::RouteRequester {
                    user,
                    is_gm: false,
                    world_role: WorldRole::Player,
                    world_defaults: &WorldCapDefaults::default(),
                    explored: None,
                },
                scene,
                path[0],
                &[path[1]],
                crate::scene::RouteMover {
                    footprint_radius: fp,
                    budget_cells: None,
                    traits: MoveTraits::default(),
                },
            );
            if let Ok(r) = route {
                assert!(
                    r.path.last().copied() != Some(path[1]),
                    "the gate refuses {:?} but a route reaches it — the gate is more \
                     permissive than the router",
                    path
                );
            }
        }
    }
}

#[test]
fn a_default_footprint_step_across_a_wall_is_truncated() {
    // The center-to-center `segments_cross` test is load-bearing here, not redundant with the
    // disc test: a wall between two adjacent cell centers sits 0.5 cell from each, so the
    // 0.4-radius disc test alone would pass it.
    let (ecs, scene, token, user) = scene_with_wall_between_adjacent_cells_and_default_footprint();
    let mask = ecs.visible_cells(
        user,
        WorldRole::Player,
        &WorldCapDefaults::default(),
        scene,
        false,
    );
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &mask,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(50.0, 50.0), (150.0, 50.0)],
        false,
        0.4,
    )
    .expect("admissible");
    assert!(out.truncated, "the wall blocks a default-footprint step");
}

#[test]
fn a_wide_token_cannot_enter_a_cell_whose_footprint_overlaps_fog() {
    let (ecs, scene, token, user) = scene_with_lit_center_line_only();
    let fp = ecs.resolve_token_footprint(token, scene).expect("in-range"); // > 0.5
    let mask = ecs.visible_cells(
        user,
        WorldRole::Player,
        &WorldCapDefaults::default(),
        scene,
        false,
    );
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &mask,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(50.0, 50.0), (150.0, 50.0)],
        false,
        fp,
    )
    .expect("admissible");
    assert!(
        out.truncated,
        "a footprint cell outside the mask stops a wide token"
    );
}

#[test]
fn a_sub_half_cell_footprint_diagonal_is_admissible() {
    // A footprint disc smaller than half a cell clears both corner flankers of a diagonal
    // step in a fully-lit area, so the step is admissible.
    let (ecs, scene, token, user) = scene_with_open_lit_area();
    let mask = ecs.visible_cells(
        user,
        WorldRole::Player,
        &WorldCapDefaults::default(),
        scene,
        false,
    );
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &mask,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(50.0, 50.0), (150.0, 150.0)],
        false,
        0.4,
    )
    .expect("admissible");
    assert!(!out.truncated, "a 0.4-radius diagonal step is allowed");
}

#[test]
fn arrest_stays_center_cell_matching_the_router() {
    // `cell_enterable` does NOT footprint-gate arrest. A wide
    // token whose FOOTPRINT touches an arrest cell but whose CENTER does not must not be
    // arrested, or the gate becomes stricter than the router and route-gate parity breaks.
    let (ecs, scene, token, user) = scene_with_arrest_cell_beside_the_path_and_wide_token();
    let fp = ecs.resolve_token_footprint(token, scene).expect("in-range"); // > 0.5
    let mask = ecs.visible_cells(
        user,
        WorldRole::Player,
        &WorldCapDefaults::default(),
        scene,
        false,
    );
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &mask,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(50.0, 50.0), (150.0, 50.0)],
        false,
        fp,
    )
    .expect("admissible");
    assert!(
        !out.truncated,
        "arrest is center-cell only, matching the router"
    );
}

#[test]
fn a_gm_is_exempt_from_every_footprint_check() {
    let (ecs, scene, token, _user, start, goal) =
        scene_with_narrow_gap_and_wide_token("square", MovementModel::GridStepped);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Unrestricted,
            visible: &empty_mask(),
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[start, goal],
        true,
        5.0,
    )
    .expect("admissible");
    assert!(
        !out.truncated,
        "a GM squeezes a wide token through anything"
    );
}

#[test]
fn execute_move_refuses_an_out_of_range_footprint() {
    // This gate input gets an admissibility guard like every other. A NaN radius
    // would make every `dist < r_scene` comparison false — fail-open.
    let (ecs, scene, token) = scene_with_wall_across_the_path_for_footprint_guard();
    for bad in [
        f64::NAN,
        -1.0,
        crate::scene::pathfinding::MAX_FOOTPRINT_CELLS + 1.0,
    ] {
        let err = execute_move(
            &ecs,
            MoveGateInputs {
                scene,
                restriction: MovementRestriction::Unrestricted,
                visible: &empty_mask(),
                cell: FIXTURE_GRID_SIZE,
                budget: None,
                traits: MoveTraits::default(),
            },
            token,
            &[(50.0, 50.0), (150.0, 50.0)],
            false,
            bad,
        )
        .expect_err("an out-of-range footprint is refused");
        assert!(
            matches!(err, MoveReject::Degenerate),
            "bad={bad}: got {err:?}"
        );
    }
}

// -----------------------------------------------------------------------
// MoveOutcome::entered_cells — the trigger fire sites' entered-cell report
// -----------------------------------------------------------------------

#[test]
fn entered_cells_reports_each_transition_once_in_order() {
    let (ecs, scene, token) = clear_scene();
    let visible = visible_grid(3);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .unwrap();
    // The origin cell is not "entered"; each subsequent cell appears exactly
    // once, in walk order, however many dense sub-samples crossed into it.
    assert_eq!(out.entered_cells, vec![(1, 0), (1, 1)]);
    assert!(!out.arrested);
}

#[test]
fn entered_cells_reports_a_revisit_as_a_second_entry() {
    let (ecs, scene, token) = clear_scene();
    let visible = visible_grid(3);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (100.0, 0.0), (0.0, 0.0)],
        false,
        0.4,
    )
    .unwrap();
    // Dedup is across a single cell's dense sub-samples, never across the
    // walk: leaving and re-entering a cell reports both entries.
    assert_eq!(out.entered_cells, vec![(1, 0), (0, 0)]);
}

#[test]
fn entered_cells_include_the_arrest_cell_and_set_the_arrest_flag() {
    let scene_id = Uuid::from_u128(10);
    let token_id = Uuid::from_u128(11);
    let ecs = SceneEcs::from_documents(
        vec![
            entity_doc(
                10,
                0,
                "scene",
                json!({ "grid": { "kind": "square", "size": FIXTURE_GRID_SIZE }, "background": null }),
            ),
            entity_doc(
                11,
                10,
                "token",
                json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            ),
            region_doc(12, 10, "arrest", 1.0, (50.0, -50.0, 150.0, 50.0)),
        ],
        0,
    );
    let visible = visible_grid(3);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene: scene_id,
            restriction: MovementRestriction::Unrestricted,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token_id,
        &[(0.0, 0.0), (100.0, 0.0)],
        false,
        0.4,
    )
    .unwrap();
    // The arrest cell counts as entered (its cost was accrued, the mover's
    // center rests there) so an `arrest`-trigger region can resolve its own
    // membership from the report alone.
    assert_eq!(out.entered_cells, vec![(1, 0)]);
    assert!(out.arrested);
}

#[test]
fn entered_cells_exclude_a_cell_the_walk_was_stopped_before() {
    let (ecs, scene, token) = walled_scene();
    let visible = visible_grid(4);
    let out = execute_move(
        &ecs,
        MoveGateInputs {
            scene,
            restriction: MovementRestriction::Visible,
            visible: &visible,
            cell: FIXTURE_GRID_SIZE,
            budget: None,
            traits: MoveTraits::default(),
        },
        token,
        &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)],
        false,
        0.4,
    )
    .unwrap();
    // The wall blocks (100,0)→(100,100): the blocked cell is never entered,
    // and a wall stop is not an arrest.
    assert_eq!(out.entered_cells, vec![(1, 0)]);
    assert!(!out.arrested);
}

mod unified_cost;
