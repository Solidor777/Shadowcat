//! Tests for `scene::senses` — the creature-sense `perceived` channel.

use super::*;
use crate::data::document::{DocRole, WorldRole};
use crate::scene::compute_derived;
use serde_json::json;

/// Builds a world-scoped fixture document of type `ty`, parented to `parent` when given.
fn doc(id: u128, parent: Option<u128>, ty: &str) -> Document {
    let mut d =
        crate::data::document::tests::world_scoped_doc(Uuid::from_u128(9), Uuid::from_u128(id), ty);
    d.parent_id = parent.map(Uuid::from_u128);
    d
}

/// Builds a scene-entity fixture with `engine` set to `body`.
fn entity_doc_eng(id: u128, parent: u128, ty: &str, body: serde_json::Value) -> Document {
    let mut d = doc(id, Some(parent), ty);
    d.engine = Some(body);
    d
}

/// A player `PermissionContext` over the given user id.
fn player_ctx(user: Uuid) -> PermissionContext {
    PermissionContext {
        user_id: user,
        world_role: WorldRole::Player,
    }
}

/// Empty world-level capability grants.
fn no_grants() -> WorldCapDefaults {
    WorldCapDefaults::default()
}

/// A minimal, structurally-complete `ActorEngine` body with `vision` set to the caller's
/// assignment array — the sense tests only ever vary `vision`.
fn actor_body(vision: serde_json::Value) -> serde_json::Value {
    json!({
        "displayName": "Fixture Actor",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "conditions": [],
        "prototype": true,
        "vision": vision,
    })
}

/// An INSTANCED token (embedded actor copy) at `(x, y)` scene units carrying `vision` as its
/// actor's vision assignments, owned by `owner`, and readable by any player
/// (`PermissionSet::default` is `DocRole::None`, so a readable fixture must say so).
fn sense_token(
    id: u128,
    scene: u128,
    owner: Uuid,
    x: f64,
    y: f64,
    vision: serde_json::Value,
) -> Document {
    let mut d = entity_doc_eng(
        id,
        scene,
        "token",
        json!({ "x": x, "y": y, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    d.owner = Some(owner);
    d.permissions.default = DocRole::Observer;
    let mut a = doc(id + 1000, None, "actor");
    a.engine = Some(actor_body(vision));
    d.embedded.insert("actor".into(), vec![a]);
    d
}

/// The fixture scene id every sense test parents to.
const SCENE: u128 = 10;
/// The fixture user's id (owns the sense source unless a test says otherwise).
const USER: u128 = 7;
/// The default scene cell size in scene units (no authored grid): positions below are
/// authored in cells × this.
const CELL: f64 = 100.0;

/// The token ids `user` perceives in the fixture's one scene.
fn perceived(ecs: &SceneEcs, user: Uuid) -> Vec<Uuid> {
    // The disjointness input is the caller-computed lit mask, exactly as `compute_derived`
    // passes it.
    let bands = ecs.resolved_bands();
    let lit = ecs.player_lit_mask(user, WorldRole::Player, &no_grants(), &bands);
    let out = ecs.player_perceived_tokens(&player_ctx(user), &no_grants(), &lit);
    assert!(out.len() <= 1, "fixture has exactly one scene");
    out.into_iter().next().map(|p| p.tokens).unwrap_or_default()
}

/// A tremorsense assignment (no authored range → inherits the mode's 12-cell default).
fn tremorsense() -> serde_json::Value {
    json!([{ "mode": "tremorsense" }])
}

#[test]
fn tremorsense_perceives_a_grounded_token_through_walls_in_darkness() {
    let source = sense_token(
        11,
        SCENE,
        Uuid::from_u128(USER),
        0.5 * CELL,
        0.5 * CELL,
        tremorsense(),
    );
    let target = sense_token(
        12,
        SCENE,
        Uuid::from_u128(8),
        3.5 * CELL,
        0.5 * CELL,
        json!([]),
    );
    // A full-blocking wall between the two, and no light anywhere: the target is invisible
    // to the terrain mask, so only the creature sense can name it.
    let wall = crate::scene::tests::wall_doc_eng(
        Uuid::from_u128(SCENE),
        (2.0 * CELL, -CELL),
        (2.0 * CELL, 2.0 * CELL),
    );
    let ecs = SceneEcs::from_documents(vec![doc(SCENE, None, "scene"), source, target, wall], 0);
    // Exactly the target — the source never perceives itself.
    assert_eq!(
        perceived(&ecs, Uuid::from_u128(USER)),
        vec![Uuid::from_u128(12)]
    );
    // Non-vacuity: the terrain mask genuinely does NOT show the target (darkness + wall).
    let lit = ecs.player_lit_mask(
        Uuid::from_u128(USER),
        WorldRole::Player,
        &no_grants(),
        &ecs.resolved_bands(),
    );
    assert!(
        lit.iter().all(|s| s.cells.is_empty()),
        "the fixture must be dark/walled enough that the lit mask names nothing"
    );
}

#[test]
fn flying_tokens_neither_perceive_nor_are_perceived() {
    // A flying TARGET (elevation ≠ ground) is not felt, while a grounded one at the same
    // distance is — proves the exclusion is the elevation, not the geometry.
    let source = sense_token(
        11,
        SCENE,
        Uuid::from_u128(USER),
        0.5 * CELL,
        0.5 * CELL,
        tremorsense(),
    );
    let mut flying = sense_token(
        12,
        SCENE,
        Uuid::from_u128(8),
        3.5 * CELL,
        0.5 * CELL,
        json!([]),
    );
    flying.engine = Some(
        json!({ "x": 3.5 * CELL, "y": 0.5 * CELL, "w": 100.0, "h": 100.0, "rotation": 0.0, "elevation": 5.0 }),
    );
    let grounded = sense_token(
        13,
        SCENE,
        Uuid::from_u128(8),
        3.5 * CELL,
        2.5 * CELL,
        json!([]),
    );
    let ecs =
        SceneEcs::from_documents(vec![doc(SCENE, None, "scene"), source, flying, grounded], 0);
    assert_eq!(
        perceived(&ecs, Uuid::from_u128(USER)),
        vec![Uuid::from_u128(13)]
    );

    // A flying SOURCE feels nothing, even with a grounded target in range.
    let mut air_source = sense_token(
        11,
        SCENE,
        Uuid::from_u128(USER),
        0.5 * CELL,
        0.5 * CELL,
        tremorsense(),
    );
    air_source.engine = Some(
        json!({ "x": 0.5 * CELL, "y": 0.5 * CELL, "w": 100.0, "h": 100.0, "rotation": 0.0, "elevation": 5.0 }),
    );
    let target = sense_token(
        13,
        SCENE,
        Uuid::from_u128(8),
        3.5 * CELL,
        0.5 * CELL,
        json!([]),
    );
    let ecs = SceneEcs::from_documents(vec![doc(SCENE, None, "scene"), air_source, target], 0);
    assert!(perceived(&ecs, Uuid::from_u128(USER)).is_empty());
}

#[test]
fn range_gates_perception_and_zero_range_is_unlimited() {
    // Default tremorsense range is 12 cells: a target 13 cells out is not perceived.
    let source = sense_token(
        11,
        SCENE,
        Uuid::from_u128(USER),
        0.5 * CELL,
        0.5 * CELL,
        tremorsense(),
    );
    let far = sense_token(
        12,
        SCENE,
        Uuid::from_u128(8),
        13.5 * CELL,
        0.5 * CELL,
        json!([]),
    );
    let ecs = SceneEcs::from_documents(vec![doc(SCENE, None, "scene"), source, far], 0);
    assert!(perceived(&ecs, Uuid::from_u128(USER)).is_empty());

    // A `range: 0` creature sense is unlimited: the same far target is perceived, and the
    // payload's token ids come out sorted (BTreeSet collection — egress change detection
    // compares whole payloads).
    let source = sense_token(
        11,
        SCENE,
        Uuid::from_u128(USER),
        0.5 * CELL,
        0.5 * CELL,
        json!([{ "mode": "tremorsense", "range": 0 }]),
    );
    let far_high_id = sense_token(
        20,
        SCENE,
        Uuid::from_u128(8),
        13.5 * CELL,
        0.5 * CELL,
        json!([]),
    );
    let far_low_id = sense_token(
        12,
        SCENE,
        Uuid::from_u128(8),
        25.5 * CELL,
        0.5 * CELL,
        json!([]),
    );
    let ecs = SceneEcs::from_documents(
        vec![doc(SCENE, None, "scene"), source, far_high_id, far_low_id],
        0,
    );
    assert_eq!(
        perceived(&ecs, Uuid::from_u128(USER)),
        vec![Uuid::from_u128(12), Uuid::from_u128(20)],
        "unlimited range reaches both, sorted by id"
    );
}

#[test]
fn a_permission_hidden_target_is_never_named() {
    let source = sense_token(
        11,
        SCENE,
        Uuid::from_u128(USER),
        0.5 * CELL,
        0.5 * CELL,
        tremorsense(),
    );
    // In range and grounded, but `default: None` with no grants: the recipient holds no
    // whole-document READ, so the creature sense must not name it (senses pierce fog, not
    // the READ gate).
    let mut hidden = sense_token(
        12,
        SCENE,
        Uuid::from_u128(8),
        3.5 * CELL,
        0.5 * CELL,
        json!([]),
    );
    hidden.permissions.default = DocRole::None;
    let readable = sense_token(
        13,
        SCENE,
        Uuid::from_u128(8),
        3.5 * CELL,
        2.5 * CELL,
        json!([]),
    );
    let ecs =
        SceneEcs::from_documents(vec![doc(SCENE, None, "scene"), source, hidden, readable], 0);
    assert_eq!(
        perceived(&ecs, Uuid::from_u128(USER)),
        vec![Uuid::from_u128(13)],
        "only the READ-able token is named"
    );
}

#[test]
fn an_already_visible_target_is_not_restated() {
    let source = sense_token(
        11,
        SCENE,
        Uuid::from_u128(USER),
        0.5 * CELL,
        0.5 * CELL,
        tremorsense(),
    );
    let target = sense_token(
        12,
        SCENE,
        Uuid::from_u128(8),
        3.5 * CELL,
        0.5 * CELL,
        json!([]),
    );
    // A light on the target's cell: the recipient's terrain mask (fallback normal vision,
    // dim floor) already shows it, so `perceived` stays disjoint by construction.
    let light = entity_doc_eng(
        20,
        SCENE,
        "light",
        json!({
            "x": 3.5 * CELL, "y": 0.5 * CELL,
            "emission": { "color": "#ffffff", "intensity": 1.0, "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }
        }),
    );
    let ecs = SceneEcs::from_documents(vec![doc(SCENE, None, "scene"), source, target, light], 0);
    // Non-vacuity: the target's center cell IS in the lit mask.
    let lit = ecs.player_lit_mask(
        Uuid::from_u128(USER),
        WorldRole::Player,
        &no_grants(),
        &ecs.resolved_bands(),
    );
    let lit_cells: std::collections::BTreeSet<(i32, i32)> = lit
        .into_iter()
        .flat_map(|s| s.cells.into_iter().map(|(i, j, ..)| (i, j)))
        .collect();
    assert!(
        lit_cells.contains(&(3, 0)),
        "the target's cell must be lit here"
    );
    assert!(
        perceived(&ecs, Uuid::from_u128(USER)).is_empty(),
        "a target the lit mask already shows is not restated in perceived"
    );
}

#[test]
fn observer_vision_admits_a_readable_non_owned_token_as_a_source() {
    let other = Uuid::from_u128(8);
    // The source belongs to someone else; the user holds READ on it (Observer default).
    let source = sense_token(11, SCENE, other, 0.5 * CELL, 0.5 * CELL, tremorsense());
    let target = sense_token(12, SCENE, other, 3.5 * CELL, 0.5 * CELL, json!([]));
    let mut ecs = SceneEcs::from_documents(vec![doc(SCENE, None, "scene"), source, target], 0);

    // observerVision OFF (the default): a non-owned token is not a source, READ or not.
    assert!(perceived(&ecs, Uuid::from_u128(USER)).is_empty());

    // observerVision ON: the same READ-able token contributes its creature sense.
    ecs.set_world_settings_for_test(crate::scene::tests::ws_body(&[(
        "/scene/observerVision",
        json!(true),
    )]));
    assert_eq!(
        perceived(&ecs, Uuid::from_u128(USER)),
        vec![Uuid::from_u128(12)]
    );
}

#[test]
fn a_requires_los_creature_sense_is_wall_bounded() {
    // An authored modes doc REPLACES the seed registry, so this sense names its own mode.
    let mut vm = doc(101, None, "vision-modes");
    vm.engine = Some(json!({ "modes": { "blindsight": {
        "id": "blindsight", "name": "Blindsight",
        "illuminationFloor": "dark", "defaultRange": 12,
        "perceives": "creatures", "requiresLos": true
    } } }));
    let source = sense_token(
        11,
        SCENE,
        Uuid::from_u128(USER),
        0.5 * CELL,
        0.5 * CELL,
        json!([{ "mode": "blindsight" }]),
    );
    // In the open, in range, in darkness: perceived (a creature sense never reads
    // illumination, but this mode still respects walls).
    let open = sense_token(
        12,
        SCENE,
        Uuid::from_u128(8),
        3.5 * CELL,
        0.5 * CELL,
        json!([]),
    );
    // Same range, but a full-blocking wall stands between source and target.
    let behind = sense_token(
        13,
        SCENE,
        Uuid::from_u128(8),
        0.5 * CELL,
        3.5 * CELL,
        json!([]),
    );
    let wall = crate::scene::tests::wall_doc_eng(
        Uuid::from_u128(SCENE),
        (-CELL, 2.0 * CELL),
        (2.0 * CELL, 2.0 * CELL),
    );
    let mut ecs = SceneEcs::from_documents(
        vec![doc(SCENE, None, "scene"), source, open, behind, wall],
        0,
    );
    ecs.set_world_config(None, None, Some(vm), None, None, None);
    assert_eq!(
        perceived(&ecs, Uuid::from_u128(USER)),
        vec![Uuid::from_u128(12)],
        "the walled-off target is not perceived by a requires-los sense"
    );
}

#[test]
fn compute_derived_carries_perceived_in_the_masked_payload_only() {
    let user = Uuid::from_u128(USER);
    let source = sense_token(11, SCENE, user, 0.5 * CELL, 0.5 * CELL, tremorsense());
    let target = sense_token(
        12,
        SCENE,
        Uuid::from_u128(8),
        3.5 * CELL,
        0.5 * CELL,
        json!([]),
    );
    // A second player with their own tremorsense source and a private far-away target —
    // the see-as shape: `compute_derived` keyed off the passed ctx must yield THEIR set.
    let seer = Uuid::from_u128(21);
    let seer_source = sense_token(13, SCENE, seer, 30.5 * CELL, 30.5 * CELL, tremorsense());
    let seer_target = sense_token(
        14,
        SCENE,
        Uuid::from_u128(8),
        33.5 * CELL,
        30.5 * CELL,
        json!([]),
    );
    let ecs = SceneEcs::from_documents(
        vec![
            doc(SCENE, None, "scene"),
            source,
            target,
            seer_source,
            seer_target,
        ],
        0,
    );

    let pv = compute_derived("vision", &ecs, &player_ctx(user), &no_grants()).unwrap();
    assert_eq!(pv["mode"], "masked");
    let perceived = pv["perceived"]
        .as_array()
        .expect("masked payload carries perceived");
    assert_eq!(perceived.len(), 1);
    assert_eq!(perceived[0]["scene"], json!(Uuid::from_u128(SCENE)));
    assert_eq!(perceived[0]["tokens"], json!([Uuid::from_u128(12)]));

    // The see-as shape: a `PermissionContext` for the other user yields their set, not
    // the first user's.
    let sv = compute_derived("vision", &ecs, &player_ctx(seer), &no_grants()).unwrap();
    assert_eq!(sv["perceived"][0]["tokens"], json!([Uuid::from_u128(14)]));

    // The GM arm carries none — a GM sees all, so there is nothing to perceive.
    let gm = PermissionContext {
        user_id: Uuid::from_u128(1),
        world_role: WorldRole::Gm,
    };
    let gv = compute_derived("vision", &ecs, &gm, &no_grants()).unwrap();
    assert_eq!(gv["mode"], "all");
    assert!(gv.get("perceived").is_none());
}
