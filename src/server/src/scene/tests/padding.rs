//! The scene margin (`SceneBounds::padding_cells`): the envelope, routing and cache behaviour it
//! adds on top of the authored play-area rectangle.
use super::*;
use crate::scene::pathfinding::MoveTraits;

/// World settings for a continuous (navmesh) scene with full environment light and no movement
/// restriction, so the only thing that can refuse a route is the mesh's own extent.
fn continuous_open_world() -> serde_json::Value {
    json!({
        "scene": {
            "losRestriction": false, "fog": true,
            "lightingEnabled": true, "lightMode": "environmentLight",
            "environment": { "color": "#ffffff", "intensity": 1.0 },
            "observerVision": false,
            "movementRestriction": "unrestricted",
            "movementModel": "continuous",
            "partialCellLeniency": true
        },
        "pathfinding": { "diagonalRule": "chebyshev" },
        "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
    })
}

/// A continuous square scene (id 10), cell size 100, 4 × 4 cells, with `padding` cells of margin
/// when given.
fn padded_scene_ecs(padding: Option<f64>) -> SceneEcs {
    let mut body = json!({
        "grid": { "kind": "square", "size": 100 }, "background": null,
        "bounds": { "width": 4.0, "height": 4.0 },
        "vision": { "movementModel": "continuous" }
    });
    if let Some(p) = padding {
        body["padding"] = json!(p);
    }
    let mut ecs = SceneEcs::from_documents(vec![entity_doc_top_eng(10, "scene", body)], 0);
    ecs.set_world_settings_for_test(continuous_open_world());
    ecs
}

/// GM route from the middle of cell (0,0) to `dest`.
fn gm_route(
    ecs: &SceneEcs,
    dest: (f64, f64),
) -> Result<pathfinding::PathOutcome, pathfinding::PathFail> {
    ecs.pathfind(
        RouteRequester {
            user: crate::test_ids::test_uuid(1),
            is_gm: true,
            world_role: WorldRole::Gm,
            world_defaults: &no_world_grants(),
            explored: None,
        },
        crate::test_ids::test_uuid(10),
        (50.0, 50.0),
        &[dest],
        crate::scene::RouteMover {
            footprint_radius: 0.1,
            budget_cells: None,
            traits: MoveTraits::default(),
            elevation: elevation::GROUND,
            token: None,
        },
    )
}

#[test]
fn a_scene_without_padding_has_the_authored_rectangle_for_its_extent() {
    // Discrimination: fails if an unauthored margin changes the envelope (the default must leave
    // every stored scene's behaviour untouched).
    let ecs = padded_scene_ecs(None);
    let e = ecs.scene_world_extent(crate::test_ids::test_uuid(10));
    assert_eq!((e.min, e.max), ((0.0, 0.0), (400.0, 400.0)));
}

#[test]
fn padding_widens_the_scene_extent_by_whole_cells_on_every_side() {
    // Discrimination: fails if the margin is applied on one side only, or in the wrong unit.
    let ecs = padded_scene_ecs(Some(2.0));
    let e = ecs.scene_world_extent(crate::test_ids::test_uuid(10));
    assert_eq!((e.min, e.max), ((-200.0, -200.0), (600.0, 600.0)));
}

#[test]
fn a_route_into_the_margin_exists_with_padding_and_is_refused_without() {
    // The destination sits one cell left of and above the origin: outside the authored rectangle,
    // inside a margin of one cell or more.
    // Discrimination: fails if the navmesh is built from the authored rectangle alone (the padded
    // route would be unreachable), and the unpadded arm fails if the margin leaks into a scene
    // that authors none.
    let dest = (-50.0, -50.0);
    let padded = padded_scene_ecs(Some(1.0));
    let out = gm_route(&padded, dest).expect("the margin is routable");
    assert_eq!(out.path.last(), Some(&dest));
    assert!(gm_route(&padded_scene_ecs(None), dest).is_err());
}

#[test]
fn changing_only_the_padding_rebuilds_the_cached_navmesh() {
    // Discrimination: fails if the navmesh cache keeps serving the mesh built for the old margin
    // after only `/engine/padding` changed.
    let mut ecs = padded_scene_ecs(None);
    let dest = (-50.0, -50.0);
    assert!(gm_route(&ecs, dest).is_err(), "primes the cache unpadded");
    ecs.apply_op(&crate::data::command::Operation::Update {
        doc_id: crate::test_ids::test_uuid(10),
        changes: vec![fc("/engine/padding", json!(1.0))],
    });
    assert!(gm_route(&ecs, dest).is_ok());
    ecs.apply_op(&crate::data::command::Operation::Update {
        doc_id: crate::test_ids::test_uuid(10),
        changes: vec![fc("/engine/padding", json!(0.0))],
    });
    assert!(gm_route(&ecs, dest).is_err());
}

#[test]
fn the_resolved_settings_differ_when_only_the_padding_differs() {
    // The visibility and lit-mask caches compare the whole `ResolvedScene`.
    // Discrimination: fails if `padding_cells` is left out of `SceneBounds`' equality.
    let a = padded_scene_ecs(None).resolve_scene(crate::test_ids::test_uuid(10));
    let b = padded_scene_ecs(Some(1.0)).resolve_scene(crate::test_ids::test_uuid(10));
    assert_ne!(a, b);
}
