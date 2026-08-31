use super::*;

#[test]
fn document_carries_name_and_engine_and_rejects_modules_key() {
    let json = serde_json::json!({
        "id": Uuid::from_u128(1), "scope": {"kind": "world", "world_id": Uuid::from_u128(9)},
        "doc_type": "token", "schema_version": 1,
        "name": "Goblin", "engine": {"x": 1.0},
        "system": {}, "created_at": 0, "updated_at": 0
    });
    let doc: Document = serde_json::from_value(json).unwrap();
    assert_eq!(doc.name.as_deref(), Some("Goblin"));
    assert!(doc.engine.is_some());

    // absent name/engine default to None (serde default)
    let bare = serde_json::json!({
        "id": Uuid::from_u128(1), "scope": {"kind": "world", "world_id": Uuid::from_u128(9)},
        "doc_type": "note", "schema_version": 1, "system": {}, "created_at": 0, "updated_at": 0
    });
    let doc: Document = serde_json::from_value(bare).unwrap();
    assert!(doc.name.is_none() && doc.engine.is_none());

    // Unknown root key `modules` is rejected: reserved for future module-scoped storage.
    let with_modules = serde_json::json!({
        "id": Uuid::from_u128(1), "scope": {"kind": "world", "world_id": Uuid::from_u128(9)},
        "doc_type": "note", "schema_version": 1, "system": {}, "modules": {},
        "created_at": 0, "updated_at": 0
    });
    assert!(serde_json::from_value::<Document>(with_modules).is_err());
}

#[test]
fn empty_schema_is_any_and_round_trips() {
    let s: super::Schema = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(s.ty.is_none() && s.properties.is_none() && s.additional_properties.is_none());
    assert_eq!(serde_json::to_value(&s).unwrap(), serde_json::json!({}));
}

#[test]
fn object_schema_deserializes_with_camel_case_additional_properties() {
    let s: super::Schema = serde_json::from_value(serde_json::json!({
        "type": "object",
        "required": ["kind"],
        "properties": { "kind": { "type": "string" }, "base": { "type": "number", "nullable": true } },
        "additionalProperties": { "type": "object" }
    }))
    .unwrap();
    assert_eq!(s.ty, Some(super::SchemaType::Object));
    assert!(matches!(
        s.additional_properties,
        Some(super::AdditionalProperties::Schema(_))
    ));
}

#[test]
fn additional_properties_accepts_bool() {
    let s: super::Schema = serde_json::from_value(serde_json::json!({
        "type": "object", "additionalProperties": true
    }))
    .unwrap();
    assert!(matches!(
        s.additional_properties,
        Some(super::AdditionalProperties::Bool(true))
    ));
}

#[test]
fn unknown_schema_key_fails_to_deserialize() {
    // deny_unknown_fields at the top level.
    assert!(serde_json::from_value::<super::Schema>(serde_json::json!({
        "type": "string", "minLength": 3
    }))
    .is_err());
}

#[test]
fn unknown_key_nested_in_additional_properties_schema_fails_to_deserialize() {
    // The custom AdditionalProperties Deserialize preserves deny_unknown_fields
    // on the inner Schema (MapAccessDeserializer, not a buffered Content), so a
    // smuggled key inside an additionalProperties subschema is REJECTED, not
    // silently dropped (mirrors the TokenVisual tagged-enum hole).
    assert!(serde_json::from_value::<super::Schema>(serde_json::json!({
        "type": "object",
        "additionalProperties": { "type": "string", "enum": ["a"] }
    }))
    .is_err());
}

#[test]
fn bad_schema_type_fails_to_deserialize() {
    assert!(
        serde_json::from_value::<super::Schema>(serde_json::json!({ "type": "integer" })).is_err()
    );
}

#[test]
fn schema_declaration_round_trips_and_rejects_unknown_field() {
    let d: super::SchemaDeclaration = serde_json::from_value(serde_json::json!({
        "module_id": "example-system", "version": "1.0.0", "schema_format": 1,
        "doc_type": "actor", "subtree_pointer": "/system/stats",
        "schema": { "type": "object" }
    }))
    .unwrap();
    assert_eq!(d.module_id, "example-system");
    let s = serde_json::to_string(&d).unwrap();
    let back: super::SchemaDeclaration = serde_json::from_str(&s).unwrap();
    assert_eq!(d, back);
    // deny_unknown_fields on the declaration envelope.
    assert!(
        serde_json::from_value::<super::SchemaDeclaration>(serde_json::json!({
            "module_id": "n", "version": "1", "schema_format": 1, "doc_type": "actor",
            "subtree_pointer": "/system/x", "schema": {}, "bogus": 1
        }))
        .is_err()
    );
}

#[test]
fn grants_for_merges_all_and_by_type() {
    let mut d = WorldCapDefaults::default();
    d.all
        .by_role
        .entry(DocRole::Owner)
        .or_default()
        .insert("core:manage_embedded".into());
    d.by_type
        .entry("token".into())
        .or_default()
        .by_role
        .entry(DocRole::Owner)
        .or_default()
        .insert("dnd5e:move".into());

    let g = d.grants_for("token");
    let owner = g.by_role.get(&DocRole::Owner).unwrap();
    assert!(owner.contains("core:manage_embedded") && owner.contains("dnd5e:move"));
    // A type with no override gets only `all`.
    assert!(!d
        .grants_for("actor")
        .by_role
        .get(&DocRole::Owner)
        .unwrap()
        .contains("dnd5e:move"));
}

#[test]
fn role_has_checks_all_and_by_type() {
    let mut d = WorldCapDefaults::default();
    d.role_caps
        .by_type
        .entry("token".into())
        .or_default()
        .entry(WorldRole::Player)
        .or_default()
        .insert("core:create".into());
    assert!(d.role_has(WorldRole::Player, "token", "core:create"));
    assert!(!d.role_has(WorldRole::Player, "actor", "core:create"));
    assert!(!d.role_has(WorldRole::Spectator, "token", "core:create"));
}

/// A minimal valid `actor` document; shared by data/validation/scene unit
/// tests that just need a well-formed baseline to mutate.
pub(crate) fn sample_doc() -> Document {
    Document {
        id: Uuid::from_u128(1),
        scope: Scope::World {
            world_id: Uuid::from_u128(9),
        },
        doc_type: "actor".to_string(),
        schema_version: 1,
        name: None,
        source: Some(Source {
            id: Uuid::from_u128(2),
            pack: Some("dnd5e".into()),
            version: 3,
        }),
        base: None,
        owner: Some(Uuid::from_u128(5)),
        permissions: PermissionSet::default(),
        embedded: BTreeMap::new(),
        parent_id: None,
        engine: None,
        system: serde_json::json!({ "hp": 10 }),
        created_at: 100,
        updated_at: 100,
    }
}

/// A world-scoped document with the given id/type and no parent; shared by
/// data, scene, and ws unit tests.
pub(crate) fn world_scoped_doc(world_id: Uuid, id: Uuid, doc_type: &str) -> Document {
    let mut d = sample_doc();
    d.id = id;
    d.scope = Scope::World { world_id };
    d.doc_type = doc_type.to_string();
    d.source = None;
    d.owner = None;
    d.parent_id = None;
    d.engine = default_test_engine(doc_type);
    d
}

/// A minimal valid `engine` body for `doc_type` (mirrors
/// `data::engine::validate_engine`'s battery), `None` for a non-engine
/// doc type. `system` bodies built by shared test helpers are opaque
/// placeholders unrelated to `doc_type` (pre-dating the engine band) and
/// stay untouched — the read-path re-root that consumes `engine` instead
/// of `system` for scene/token/etc. is later checkpoint work; this only
/// satisfies the ingress gate so `apply_intent`-driven fixtures can still
/// Create/Update.
pub(crate) fn default_test_engine(doc_type: &str) -> Option<serde_json::Value> {
    match doc_type {
        "token" => Some(serde_json::json!({
            "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0
        })),
        "scene" => Some(serde_json::json!({
            "grid": { "kind": "square", "size": 100.0 }, "background": null
        })),
        "wall" => {
            Some(serde_json::json!({ "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 } }))
        }
        "region" => Some(serde_json::json!({
            "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
            "behavior": "terrain", "cost": 1.0, "enabled": true
        })),
        "light" => Some(serde_json::json!({
            "x": 0.0, "y": 0.0, "emission": { "color": "#fff", "intensity": 1.0, "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true }
        })),
        "drawing" => Some(serde_json::json!({
            "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
            "stroke": null, "fill": null
        })),
        "template" => Some(serde_json::json!({
            "shape": { "kind": "cone", "x": 0.0, "y": 0.0, "size": 5.0, "direction": 0.0 },
            "color": "#f00"
        })),
        "actor" => Some(serde_json::json!({
            "displayName": "Test", "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
            "faction": null, "conditions": [], "prototype": true
        })),
        "message" => None, // chat's own re-root builds this doc directly; see `chat::build_message_doc`
        "world-settings" => {
            Some(serde_json::to_value(crate::data::engine::WorldSettingsEngine::default()).unwrap())
        }
        "vision-modes" => Some(serde_json::json!({ "modes": {} })),
        "light-gradation" => Some(serde_json::json!({ "bands": [] })),
        "chat-settings" => Some(serde_json::json!({})),
        "dice-settings" => Some(serde_json::json!({})),
        "channel-registry" => Some(serde_json::json!({ "channels": {} })),
        "faction-registry" => Some(serde_json::json!({ "factions": {} })),
        "condition-registry" => Some(serde_json::json!({ "conditions": {} })),
        "combat" => Some(serde_json::json!({
            "scene_id": "00000000-0000-0000-0000-000000000001",
            "active": false, "round": 0, "turn": null, "turn_control": "owner_may_end", "order": [],
            "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" },
            "effect_cleanup": true, "rewind_restore": true, "forward_restore": false,
            "effect_lifecycle": { "onCombatEnd": null, "onTurnEnd": null, "onAdvance": null }
        })),
        "combatant" => Some(serde_json::json!({
            "kind": { "type": "event", "lifespan": null, "message": null },
            "initiative": null, "tiebreak": 0.0, "resources": {}
        })),
        "resource-registry" => Some(serde_json::json!({ "resources": {} })),
        "effect" => {
            Some(serde_json::json!({ "active": true, "transfer": false, "duration": null }))
        }
        "system-defaults" => Some(serde_json::json!({})),
        "combat-history" => Some(serde_json::json!({ "records": [], "cursor": 0 })),
        _ => None,
    }
}

#[test]
fn document_round_trips_through_json() {
    let doc = sample_doc();
    let s = serde_json::to_string(&doc).unwrap();
    let back: Document = serde_json::from_str(&s).unwrap();
    assert_eq!(doc, back);
}

#[test]
fn unknown_envelope_field_is_rejected() {
    let mut value = serde_json::to_value(sample_doc()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("bogus".into(), serde_json::json!(1));
    let err = serde_json::from_value::<Document>(value);
    assert!(
        err.is_err(),
        "deny_unknown_fields should reject the bogus key"
    );
}

#[test]
fn permissionset_default_role_is_none() {
    assert_eq!(PermissionSet::default().default, DocRole::None);
}

#[test]
fn document_round_trips_base_snapshot_and_defaults_none() {
    // base defaults to None when absent (serde default).
    let bare = serde_json::json!({
        "id": Uuid::from_u128(1), "scope": {"kind": "world", "world_id": Uuid::from_u128(9)},
        "doc_type": "actor", "schema_version": 1, "system": {}, "created_at": 0, "updated_at": 0
    });
    let doc: Document = serde_json::from_value(bare).unwrap();
    assert!(doc.base.is_none());

    // A present base round-trips verbatim, even holding an engine shape that is
    // invalid for the current doc_type (base is an opaque historical snapshot).
    let mut with_base = sample_doc();
    with_base.base = Some(serde_json::json!({
        "name": "Old", "engine": { "not": "a-valid-token-engine" },
        "system": { "hp": 1 }, "embedded": {}
    }));
    let s = serde_json::to_string(&with_base).unwrap();
    let back: Document = serde_json::from_str(&s).unwrap();
    assert_eq!(with_base, back);
}
