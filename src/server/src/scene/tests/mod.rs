//! Shared fixtures for the `scene` module's test suite, split by subject into sibling files.
//!
//! Every helper, constant, and the test-only `SceneEcs` extension impl below is used by two or
//! more of the three subject files (`ecs_and_footprints`, `resolution_and_lighting`,
//! `pathfind_and_vision`) and therefore lives here rather than being duplicated.
pub(super) use super::*;
pub(super) use crate::data::document::WorldCapDefaults;
pub(super) use grid_shape::GridShape as _;
pub(super) use serde_json::json;

/// Builds a world-scoped fixture document of type `ty`, parented to `parent` when given.
pub(super) fn doc(id: u128, parent: Option<u128>, ty: &str) -> Document {
    let mut d =
        crate::data::document::tests::world_scoped_doc(Uuid::from_u128(9), Uuid::from_u128(id), ty);
    d.parent_id = parent.map(Uuid::from_u128);
    d
}

/// The one value read at every site in this module where a hex scene's declared `grid.size`
/// and the `HexGrid` a test derives COORDINATES from have to be the SAME number: the scene's
/// own `grid.size`, the shape the test builds, and any gate `cell` it passes alongside them.
/// A test whose expectations come from `cell_center`/`cell_vertices` is measuring the scene it
/// declared only while those agree, and nothing else makes them agree.
///
/// That scope is a PREDICATE, not a list, so a hex scene outside it is outside it because the
/// predicate does not hold, never by exemption. Two are worth naming only because they read
/// like members. The two `resolve_grid_shape_*` tests do build a shape and compare cell
/// centres, but their subject is that shape resolution takes its SIZE from the caller's
/// parameter and never from the document, so the declared size and the compared shape are
/// required to be INDEPENDENT — binding them would assert away the property under test, and a
/// parameter/expectation mismatch fails their `cell_center` comparison outright rather than
/// silently.
pub(super) const HEX_FIXTURE_SIZE: f64 = 50.0;

/// Builds a scene-entity fixture with `engine` set to `body` (`system` stays `{}`), used by
/// every fixture whose doc_type the `scene` module's production code reads through
/// `engine_as`/a typed `*Engine` struct — every derivation reader there, `token_move`
/// included (movement position lives exclusively in `/engine`).
pub(super) fn entity_doc_eng(
    id: u128,
    parent: u128,
    ty: &str,
    body: serde_json::Value,
) -> Document {
    let mut d = doc(id, Some(parent), ty);
    d.engine = Some(body);
    d
}

/// World-scoped (parentless) counterpart of `entity_doc_eng`, for config-docs
/// (`world-settings`/`vision-modes`/`light-gradation`) and `actor` docs.
pub(super) fn entity_doc_top_eng(id: u128, ty: &str, body: serde_json::Value) -> Document {
    let mut d = doc(id, None, ty);
    d.engine = Some(body);
    d
}

/// A minimal, structurally-complete `ActorEngine` body (`displayName`/`visual`/`size`/
/// `shape`/`conditions`/`prototype` are all required, non-`Option` fields) with `vision` set
/// to the caller's assignment array — the vision-floor tests only ever vary `vision`.
pub(super) fn actor_body(vision: serde_json::Value) -> serde_json::Value {
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

/// Builds a non-removing `FieldChange` setting `path` to `new` (`old` is an unused placeholder).
pub(super) fn fc(path: &str, new: serde_json::Value) -> crate::data::command::FieldChange {
    crate::data::command::FieldChange {
        remove: false,
        path: path.into(),
        old: json!(0),
        new,
    }
}

// --- Test helpers for movement-restriction resolution tests ---

/// Set `world_settings` to a doc whose `engine` is `json_engine` (test-only).
/// Mirrors `document::tests::world_scoped_doc`, the same test helper `ws::room`'s own tests
/// use to build a world-settings config doc.
#[cfg(test)]
impl SceneEcs {
    pub(crate) fn set_world_settings_for_test(&mut self, json_engine: serde_json::Value) {
        let mut d = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::from_u128(100),
            "world-settings",
        );
        d.engine = Some(json_engine);
        self.world_settings = Some(d);
    }

    /// Set `system_defaults` to a doc whose `engine` is `json_engine` (test-only). Mirrors
    /// `set_world_settings_for_test`'s shape for the `system-defaults` singleton side-table.
    pub(crate) fn set_system_defaults_for_test(&mut self, json_engine: serde_json::Value) {
        let mut d = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::from_u128(101),
            "system-defaults",
        );
        d.engine = Some(json_engine);
        self.system_defaults = Some(d);
    }

    pub(crate) fn insert_scene_for_test(&mut self, scene_id: Uuid, json_engine: serde_json::Value) {
        let mut d =
            crate::data::document::tests::world_scoped_doc(Uuid::from_u128(9), scene_id, "scene");
        d.engine = Some(json_engine);
        // Remove stale entity if re-inserting.
        if let Some(old_e) = self.index.remove(&scene_id) {
            let _ = self.world.despawn(old_e);
        }
        let e = self.world.spawn((SceneEntity { doc: d },));
        self.index.insert(scene_id, e);
    }
}

/// A COMPLETE `WorldSettingsEngine` body (all `WorldSceneDefaults` fields present, per the
/// ingress-validated struct's `deny_unknown_fields` contract) with `patches` applied over the
/// built-in default via JSON-pointer `set_pointer` — lets each test express only the field(s)
/// it cares about instead of re-typing the full 9-field `scene` object every time.
pub(super) fn ws_body(patches: &[(&str, serde_json::Value)]) -> serde_json::Value {
    use crate::data::command::set_pointer;
    let mut v = serde_json::to_value(eng::WorldSettingsEngine::default()).unwrap();
    for (path, val) in patches {
        let _ = set_pointer(&mut v, path, val.clone());
    }
    v
}

/// Builds a SceneEcs with one scene (id 10), one player-owned token at (50, 50), and one
/// enabled white light at (50, 50) with bright=3 / dim=6 cells. The token has normal vision
/// (default), so cells within the lit radius are visible. Returns `(ecs, user, scene_id)`.
pub(super) fn scene_with_lit_player_token() -> (SceneEcs, Uuid, Uuid) {
    let user = Uuid::from_u128(7);
    let scene_id = Uuid::from_u128(10);
    let mut tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    tok.owner = Some(user);
    let light = entity_doc_eng(
        20,
        10,
        "light",
        json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
        }),
    );
    let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, light], 0);
    (ecs, user, scene_id)
}

/// Gate-vs-egress parity helper: asserts `visible_cells(user, scene, false)` == the `(i,j)` set of
/// `player_lit_mask(user)` filtered to `scene`, and that neither set is empty (non-vacuous).
pub(super) fn assert_strict_parity(ecs: &SceneEcs, user: Uuid, scene: Uuid) {
    let strict: std::collections::BTreeSet<(i32, i32)> = ecs.visible_cells(user, scene, false);
    let egress: std::collections::BTreeSet<(i32, i32)> = ecs
        .player_lit_mask(user)
        .into_iter()
        .filter(|s| s.scene == scene)
        .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
        .collect();
    assert!(
        !strict.is_empty(),
        "parity check must be non-vacuous (strict set empty)"
    );
    assert!(
        !egress.is_empty(),
        "parity check must be non-vacuous (egress set empty)"
    );
    assert_eq!(
        strict, egress,
        "strict gate mask must equal the egress secrecy mask"
    );
}

/// The `(i,j)` cell set of `player_lit_mask(user)` restricted to `scene`.
pub(super) fn mask_cells(
    ecs: &SceneEcs,
    user: Uuid,
    scene: Uuid,
) -> std::collections::BTreeSet<(i32, i32)> {
    ecs.player_lit_mask(user)
        .into_iter()
        .filter(|s| s.scene == scene)
        .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
        .collect()
}

/// A `wall` doc parented to `scene`, blocksMove+blocksSight+blocksLight all true.
pub(super) fn wall_doc_eng(scene: Uuid, a: (f64, f64), b: (f64, f64)) -> Document {
    let mut d =
        crate::data::document::tests::world_scoped_doc(Uuid::from_u128(9), Uuid::new_v4(), "wall");
    d.parent_id = Some(scene);
    d.engine = Some(json!({
        "seg": { "x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1 },
        "blocksMove": true,
        "blocksSight": true,
        "blocksLight": true,
    }));
    d
}

/// A `region` doc fixture. Mirrors `move_exec::tests::region_doc` — duplicated here rather than
/// made `pub(crate)` there, to keep the two test modules independent.
pub(super) fn region_doc(
    id: u128,
    parent: u128,
    behavior: &str,
    cost: f64,
    rect: (f64, f64, f64, f64),
) -> Document {
    let (x0, y0, x1, y1) = rect;
    entity_doc_eng(
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

mod cost_parity;
mod ecs_and_footprints;
mod pathfind_and_vision;
mod resolution_and_lighting;
mod system_defaults_layer;
