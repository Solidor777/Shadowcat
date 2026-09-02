//! Repository tests, split by subject; shared fixtures live here.

mod assets;
mod combat_batches;
mod commands_and_intents;
mod invites_and_ownership;
mod moves;
mod rows_and_validation;
mod search_and_worlds;

pub(super) use super::*;
pub(super) use crate::data::command::FieldChange;
pub(super) use crate::data::document::Source;

/// Opens a fresh in-memory repository for one test.
pub(super) async fn repo() -> SqliteRepository {
    SqliteRepository::connect("sqlite::memory:").await.unwrap()
}

/// A world-scoped actor document with the given permissions and system body.
/// Callers overwrite `scope` with the real world id.
pub(super) fn tests_doc(
    perms: crate::data::document::PermissionSet,
    system: serde_json::Value,
) -> Document {
    Document {
        id: Uuid::new_v4(),
        scope: Scope::World {
            world_id: Uuid::from_u128(9),
        },
        doc_type: "actor".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: perms,
        embedded: Default::default(),
        parent_id: None,
        // "actor" is engine-defined; a minimal valid body so `Create`
        // clears the ingress gate. Unrelated to `system` (opaque,
        // caller-supplied) — this helper predates the engine band.
        engine: crate::data::document::tests::default_test_engine("actor"),
        system,
        created_at: 0,
        updated_at: 0,
    }
}

/// A world-scoped document of `doc_type` carrying an `engine` body
/// (no `system` content — `engine`-typed docs in this battery don't
/// need one). Callers overwrite `scope` with the real world id.
pub(super) fn tests_engine_doc(
    perms: crate::data::document::PermissionSet,
    doc_type: &str,
    engine: serde_json::Value,
) -> Document {
    let mut d = tests_doc(perms, serde_json::json!({}));
    d.doc_type = doc_type.into();
    d.engine = Some(engine);
    d
}

/// A world-scoped document with `doc_type` fixed to `"actor"`. Callers that
/// override `doc_type` afterward must also recompute `engine` for the new
/// type.
pub(super) fn world_doc(id: u128, world: Uuid, system: serde_json::Value) -> Document {
    Document {
        id: Uuid::from_u128(id),
        scope: Scope::World { world_id: world },
        doc_type: "actor".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: Default::default(),
        embedded: Default::default(),
        parent_id: None,
        // "actor" is engine-defined; a minimal valid body so `Create`
        // clears the ingress gate. Callers that override `doc_type`
        // afterward must also recompute `engine` for the new type.
        engine: crate::data::document::tests::default_test_engine("actor"),
        system,
        created_at: 0,
        updated_at: 0,
    }
}

/// A world-scoped document of `doc_type` with a valid `engine` body for
/// singleton create-gate tests. Mirrors `world_doc`/`tests_engine_doc`
/// but lets the caller pick `doc_type` (needed for the singleton types,
/// which `world_doc` hardcodes to "actor").
pub(super) fn singleton_test_doc(id: u128, world: Uuid, doc_type: &str) -> Document {
    let mut d = world_doc(id, world, serde_json::json!({}));
    d.doc_type = doc_type.into();
    d.engine = crate::data::document::tests::default_test_engine(doc_type);
    d
}

/// A world-scoped `token` doc, optionally linked to `actor_id`. `permissions`
/// deliberately stays at the `buildTokenDoc` shipping default (`default:
/// Observer`, no per-user entry) — the whole point is that write authority
/// comes from effective ownership, not from a stamped permission entry.
pub(super) fn owned_token_doc(world: Uuid, actor_id: Option<Uuid>) -> Document {
    use crate::data::document::{DocRole, PermissionSet, Scope};
    let mut engine = serde_json::json!({
        "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0
    });
    if let Some(a) = actor_id {
        engine["actor_id"] = serde_json::json!(a.to_string());
    }
    let mut d = tests_engine_doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        "token",
        engine,
    );
    d.scope = Scope::World { world_id: world };
    d
}

/// A world-scoped `actor` doc owned by `owner`.
pub(super) fn actor_doc_owned_by(world: Uuid, owner: Option<Uuid>) -> Document {
    use crate::data::document::{DocRole, PermissionSet, Scope};
    let mut d = tests_doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({}),
    );
    d.scope = Scope::World { world_id: world };
    d.owner = owner;
    d
}

/// A `combat` document bound to `scene`, `active` as given.
pub(super) fn combat_doc(id: u128, world: Uuid, scene: Uuid, active: bool) -> Document {
    let mut d = world_doc(id, world, serde_json::json!({}));
    d.doc_type = "combat".into();
    d.engine = Some(serde_json::json!({
        "scene_id": scene.to_string(), "active": active, "round": 0, "turn": null,
        "turn_control": "owner_may_end", "order": [],
        "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" },
        "effect_cleanup": true, "rewind_restore": true, "forward_restore": false,
        "effect_lifecycle": { "onCombatEnd": null, "onTurnEnd": null, "onAdvance": null }
    }));
    d
}

/// A `combatant` document parented to `parent`.
pub(super) fn combatant_doc(id: u128, world: Uuid, parent: Uuid) -> Document {
    let mut d = world_doc(id, world, serde_json::json!({}));
    d.doc_type = "combatant".into();
    d.parent_id = Some(parent);
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    d
}

/// An `asset_folder` document named `name` under `parent`.
pub(super) fn folder_doc(id: u128, world: Uuid, name: &str, parent: Option<Uuid>) -> Document {
    let mut d = world_doc(id, world, serde_json::json!({}));
    d.doc_type = "asset_folder".into();
    d.name = Some(name.into());
    d.parent_id = parent;
    d.engine = Some(serde_json::json!({ "sort": 0 }));
    d
}

/// A GM-owned world plus its GM `PermissionContext`.
pub(super) async fn gm_world(
    repo: &SqliteRepository,
) -> (Uuid, crate::data::membership::PermissionContext) {
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    (
        w.id,
        crate::data::membership::PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        },
    )
}
