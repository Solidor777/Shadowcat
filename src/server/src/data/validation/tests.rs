use super::*;
use uuid::Uuid;

fn doc_with_system(system: serde_json::Value) -> Document {
    Document {
        id: Uuid::from_u128(1),
        scope: crate::data::document::Scope::World {
            world_id: Uuid::from_u128(9),
        },
        doc_type: "actor".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: Default::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: None,
        system,
        created_at: 0,
        updated_at: 0,
    }
}

#[test]
fn small_system_passes() {
    assert!(validate_system_size(&doc_with_system(serde_json::json!({ "hp": 1 }))).is_ok());
}

#[test]
fn oversized_system_is_rejected() {
    let big = "x".repeat(MAX_SYSTEM_BYTES + 1);
    let err = validate_system_size(&doc_with_system(serde_json::json!({ "blob": big })));
    assert!(matches!(err, Err(DataError::TooLarge(_))));
}

#[test]
fn oversized_embedded_child_is_rejected() {
    let mut parent = doc_with_system(serde_json::json!({ "hp": 1 }));
    let child = doc_with_system(serde_json::json!({ "blob": "x".repeat(MAX_SYSTEM_BYTES + 1) }));
    parent.embedded.insert("items".into(), vec![child]);
    assert!(matches!(
        validate_system_size(&parent),
        Err(DataError::TooLarge(_))
    ));
}

#[test]
fn small_embedded_tree_passes() {
    let mut parent = doc_with_system(serde_json::json!({ "hp": 1 }));
    let child = doc_with_system(serde_json::json!({ "k": 1 }));
    parent.embedded.insert("items".into(), vec![child]);
    assert!(validate_system_size(&parent).is_ok());
}

#[test]
fn oversized_grandchild_is_rejected() {
    let mut parent = doc_with_system(serde_json::json!({}));
    let mut child = doc_with_system(serde_json::json!({}));
    let gc = doc_with_system(serde_json::json!({ "blob": "x".repeat(MAX_SYSTEM_BYTES + 1) }));
    child.embedded.insert("nested".into(), vec![gc]);
    parent.embedded.insert("items".into(), vec![child]);
    assert!(matches!(
        validate_system_size(&parent),
        Err(DataError::TooLarge(_))
    ));
}

#[test]
fn field_paths_validate() {
    assert!(validate_field_path("").is_ok());
    assert!(validate_field_path("/system/hp").is_ok());
    assert!(matches!(
        validate_field_path("system/hp"),
        Err(DataError::BadPath(_))
    ));
}

// --- per-block engine cap (mirrors the `system` battery above) ---

fn doc_with_engine(engine: serde_json::Value) -> Document {
    let mut doc = doc_with_system(serde_json::json!({}));
    doc.doc_type = "wall".into();
    doc.engine = Some(engine);
    doc
}

#[test]
fn small_engine_passes() {
    let v = serde_json::json!({ "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 } });
    assert!(validate_system_size(&doc_with_engine(v)).is_ok());
}

#[test]
fn oversized_engine_is_rejected() {
    let big = "x".repeat(MAX_SYSTEM_BYTES + 1);
    let err = validate_system_size(&doc_with_engine(serde_json::json!({ "blob": big })));
    assert!(matches!(err, Err(DataError::TooLarge(_))));
}

#[test]
fn oversized_embedded_child_engine_is_rejected() {
    let mut parent = doc_with_system(serde_json::json!({ "hp": 1 }));
    let child = doc_with_engine(serde_json::json!({ "blob": "x".repeat(MAX_SYSTEM_BYTES + 1) }));
    parent.embedded.insert("items".into(), vec![child]);
    assert!(matches!(
        validate_system_size(&parent),
        Err(DataError::TooLarge(_))
    ));
}

#[test]
fn small_embedded_engine_tree_passes() {
    let mut parent = doc_with_system(serde_json::json!({ "hp": 1 }));
    let child = doc_with_engine(serde_json::json!({ "k": 1 }));
    parent.embedded.insert("items".into(), vec![child]);
    assert!(validate_system_size(&parent).is_ok());
}

#[test]
fn oversized_engine_grandchild_is_rejected() {
    let mut parent = doc_with_system(serde_json::json!({}));
    let mut child = doc_with_system(serde_json::json!({}));
    let gc = doc_with_engine(serde_json::json!({ "blob": "x".repeat(MAX_SYSTEM_BYTES + 1) }));
    child.embedded.insert("nested".into(), vec![gc]);
    parent.embedded.insert("items".into(), vec![child]);
    assert!(matches!(
        validate_system_size(&parent),
        Err(DataError::TooLarge(_))
    ));
}

// --- validate_property_overrides: pointer-key well-formedness ---

#[test]
fn valid_property_override_keys_pass() {
    use crate::data::document::Visibility;
    let mut doc = doc_with_system(serde_json::json!({}));
    doc.permissions
        .property_overrides
        .insert("/engine".into(), Visibility::GmOnly);
    doc.permissions
        .property_overrides
        .insert("/engine/vision".into(), Visibility::GmOnly);
    doc.permissions
        .property_overrides
        .insert("/name".into(), Visibility::GmOnly);
    assert!(validate_property_overrides(&doc).is_ok());
}

#[test]
fn trailing_slash_override_key_is_rejected() {
    use crate::data::document::Visibility;
    let mut doc = doc_with_system(serde_json::json!({}));
    doc.permissions
        .property_overrides
        .insert("/engine/".into(), Visibility::GmOnly);
    assert!(matches!(
        validate_property_overrides(&doc),
        Err(DataError::BadPath(_))
    ));
}

#[test]
fn missing_leading_slash_override_key_is_rejected() {
    use crate::data::document::Visibility;
    let mut doc = doc_with_system(serde_json::json!({}));
    doc.permissions
        .property_overrides
        .insert("engine".into(), Visibility::GmOnly);
    assert!(matches!(
        validate_property_overrides(&doc),
        Err(DataError::BadPath(_))
    ));
}

#[test]
fn malformed_override_key_in_embedded_child_is_rejected() {
    use crate::data::document::Visibility;
    let mut parent = doc_with_system(serde_json::json!({}));
    let mut child = doc_with_system(serde_json::json!({}));
    child
        .permissions
        .property_overrides
        .insert("/system/secret/".into(), Visibility::GmOnly);
    parent.embedded.insert("items".into(), vec![child]);
    assert!(matches!(
        validate_property_overrides(&parent),
        Err(DataError::BadPath(_))
    ));
}

fn doc_with_override(pointer: &str) -> Document {
    let mut d = doc_with_system(serde_json::json!({ "hp": 1 }));
    d.permissions.property_overrides.insert(
        pointer.to_string(),
        crate::data::document::Visibility::GmOnly,
    );
    d
}

#[test]
fn override_naming_an_envelope_field_is_rejected() {
    for pointer in [
        "/permissions",
        "/permissions/default",
        "/permissions/users",
        "/permissions/property_overrides",
        "/owner",
        "/id",
        "/scope",
        "/doc_type",
        "/schema_version",
        "/source",
        "/parent_id",
        "/embedded",
        "/embedded/items/0",
        "/created_at",
        "/updated_at",
    ] {
        assert!(
            matches!(
                validate_property_overrides(&doc_with_override(pointer)),
                Err(DataError::BadPath(_))
            ),
            "{pointer} must be rejected at ingress"
        );
    }
}

#[test]
fn override_naming_a_content_band_is_accepted() {
    for pointer in [
        "/name",
        "/engine",
        "/engine/vision",
        "/system",
        "/system/hp",
        "/system/a/b/c",
        "/base",
        "/base/system/hp",
    ] {
        assert!(
            validate_property_overrides(&doc_with_override(pointer)).is_ok(),
            "{pointer} must be accepted at ingress"
        );
    }
}

#[test]
fn an_embedded_child_override_is_classified_too() {
    let mut parent = doc_with_system(serde_json::json!({}));
    let child = doc_with_override("/permissions/default");
    parent.embedded.insert("items".to_string(), vec![child]);
    assert!(matches!(
        validate_property_overrides(&parent),
        Err(DataError::BadPath(_))
    ));
}

// --- validate_engine_tree: gate + carry-forward normalization ---

fn valid_wall_engine() -> serde_json::Value {
    serde_json::json!({ "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 } })
}

#[test]
fn validate_engine_tree_accepts_valid_engine_doc_type() {
    let mut doc = doc_with_engine(valid_wall_engine());
    assert!(validate_engine_tree(&mut doc).is_ok());
}

#[test]
fn validate_engine_tree_rejects_engine_on_non_engine_doc_type() {
    let mut doc = doc_with_system(serde_json::json!({}));
    doc.doc_type = "item".into();
    doc.engine = Some(serde_json::json!({ "anything": 1 }));
    assert!(matches!(
        validate_engine_tree(&mut doc),
        Err(DataError::BadEngine(_))
    ));
}

#[test]
fn validate_engine_tree_rejects_malformed_engine_body() {
    let mut doc = doc_with_engine(serde_json::json!({ "seg": { "x1": "not-a-number" } }));
    assert!(matches!(
        validate_engine_tree(&mut doc),
        Err(DataError::BadEngine(_))
    ));
}

#[test]
fn validate_engine_tree_rejects_out_of_bound_token_position() {
    let over = crate::scene::move_exec::MAX_GATE_WALK_COORD + 1.0;
    let mut doc = doc_with_engine(serde_json::json!({
        "x": over, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0
    }));
    doc.doc_type = "token".into();
    assert!(matches!(
        validate_engine_tree(&mut doc),
        Err(DataError::BadEngine(_))
    ));
}

#[test]
fn validate_engine_tree_recurses_into_embedded_descendants() {
    let mut parent = doc_with_system(serde_json::json!({}));
    parent.doc_type = "item".into(); // non-engine parent; only the child carries engine
    let bad_child = doc_with_engine(serde_json::json!({ "seg": { "x1": "bad" } }));
    parent.embedded.insert("items".into(), vec![bad_child]);
    assert!(matches!(
        validate_engine_tree(&mut parent),
        Err(DataError::BadEngine(_))
    ));
}

#[test]
fn validate_engine_tree_normalizes_actor_faction_to_explicit_null() {
    // Carry-forward: an ingress-absent optional field (`faction`) must
    // persist as an explicit `null`, not stay absent.
    let mut doc = doc_with_system(serde_json::json!({}));
    doc.doc_type = "actor".into();
    doc.engine = Some(serde_json::json!({
        "displayName": "Goblin",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "conditions": [],
        "prototype": true
        // "faction" intentionally omitted
    }));
    validate_engine_tree(&mut doc).unwrap();
    let stored = doc.engine.unwrap();
    assert!(
        stored.get("faction").is_some(),
        "faction key must be present in the persisted engine body"
    );
    assert_eq!(stored["faction"], serde_json::Value::Null);
}

#[test]
fn validate_engine_tree_drops_unknown_keys_smuggled_into_a_tagged_enum() {
    // Carry-forward: `TokenVisual`/`RenderVisual`/`AnimatedSource` cannot
    // carry `deny_unknown_fields` (a serde limitation on tagged enums);
    // the re-serialization round trip must still drop a smuggled key.
    let mut doc = doc_with_system(serde_json::json!({}));
    doc.doc_type = "actor".into();
    doc.engine = Some(serde_json::json!({
        "displayName": "Goblin",
        "visual": { "kind": "image", "asset": "a.png", "smuggled": "evil" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "faction": null,
        "conditions": [],
        "prototype": true
    }));
    validate_engine_tree(&mut doc).unwrap();
    let stored = doc.engine.unwrap();
    assert!(
        stored["visual"].get("smuggled").is_none(),
        "unknown key smuggled into a tagged-enum sub-object must be dropped on persist"
    );
}

// --- base: independent size cap + the `MergeBase` walk ---

#[test]
fn oversized_base_is_rejected() {
    let mut doc = doc_with_system(serde_json::json!({ "hp": 1 }));
    doc.base = Some(serde_json::json!({ "blob": "x".repeat(MAX_SYSTEM_BYTES + 1) }));
    assert!(matches!(
        validate_system_size(&doc),
        Err(DataError::TooLarge(_))
    ));
}

#[test]
fn small_base_passes() {
    let mut doc = doc_with_system(serde_json::json!({ "hp": 1 }));
    doc.base = Some(serde_json::json!({ "name": "T", "system": { "hp": 1 } }));
    assert!(validate_system_size(&doc).is_ok());
}

/// A complete, valid `MergeBase` value for a wall-engine document; the
/// embedded collection carries one record keyed by `child_source_id`. The
/// root engine is the NORMALIZED `WallEngine` form (the optional `blocks_*`
/// fields materialize as explicit nulls through `normalize_engine_opt`'s
/// re-serialization), so a valid snapshot survives the walk untouched.
fn valid_wall_base(child_source_id: &str) -> serde_json::Value {
    serde_json::json!({
        "name": "Old",
        "engine": {
            "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 },
            "blocksSight": null, "blocksLight": null, "blocksMove": null
        },
        "system": { "hp": 1 },
        "embedded": {
            "items": [{
                "sourceId": child_source_id,
                "name": "Child",
                "engine": null,
                "system": {},
                "embedded": {},
                "propertyOverrides": {}
            }]
        },
        "property_overrides": {},
        "owner_standing": "owner"
    })
}

#[test]
fn validate_engine_tree_accepts_and_preserves_a_valid_base() {
    let mut doc = doc_with_engine(valid_wall_engine());
    let base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
    doc.base = Some(base.clone());
    validate_engine_tree(&mut doc).unwrap();
    assert_eq!(doc.base.unwrap(), base, "a valid base round-trips verbatim");
}

#[test]
fn validate_engine_tree_normalizes_the_root_base_engine_band() {
    // The base walk runs the same carry-forward normalization as the live
    // band: an ingress-absent optional field (`faction`) must persist as an
    // explicit `null` inside the snapshot too.
    let mut doc = doc_with_system(serde_json::json!({}));
    doc.doc_type = "actor".into();
    doc.engine = Some(serde_json::json!({
        "displayName": "Goblin",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "faction": null,
        "conditions": [],
        "prototype": true
    }));
    doc.base = Some(serde_json::json!({
        "name": "Old",
        "engine": {
            "displayName": "Goblin",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "conditions": [],
            "prototype": true
            // "faction" intentionally omitted
        },
        "system": {},
        "embedded": {},
        "property_overrides": {},
        "owner_standing": "owner"
    }));
    validate_engine_tree(&mut doc).unwrap();
    let base = doc.base.unwrap();
    assert_eq!(base["engine"]["faction"], serde_json::Value::Null);
}

#[test]
fn validate_engine_tree_rejects_a_non_object_base() {
    let mut doc = doc_with_engine(valid_wall_engine());
    doc.base = Some(serde_json::json!(42));
    assert!(matches!(
        validate_engine_tree(&mut doc),
        Err(DataError::SchemaViolation { .. })
    ));
}

#[test]
fn validate_engine_tree_requires_the_recorded_policy_key_at_every_depth() {
    // An absent policy map would read as "nothing hidden" — the fail-open
    // direction for the egress redaction that reads it — so the key is
    // required on the root and on every record, under each node's spelling.
    let mut doc = doc_with_engine(valid_wall_engine());
    let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
    base.as_object_mut().unwrap().remove("property_overrides");
    doc.base = Some(base);
    match validate_engine_tree(&mut doc).unwrap_err() {
        DataError::SchemaViolation { pointer, reason } => {
            assert_eq!(pointer, "/base");
            assert!(reason.contains("property_overrides"), "{reason}");
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
    let mut doc = doc_with_engine(valid_wall_engine());
    let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
    base["embedded"]["items"][0]
        .as_object_mut()
        .unwrap()
        .remove("propertyOverrides");
    doc.base = Some(base);
    match validate_engine_tree(&mut doc).unwrap_err() {
        DataError::SchemaViolation { pointer, reason } => {
            assert_eq!(pointer, "/base/embedded/items/0");
            assert!(reason.contains("propertyOverrides"), "{reason}");
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

#[test]
fn validate_engine_tree_rejects_a_recorded_policy_egress_cannot_act_on() {
    // Every recorded entry must classify once prefixed with `/base` and carry
    // a real tier, or the egress redaction reading it would fail closed on a
    // stored document.
    for (policy, what) in [
        (
            serde_json::json!({ "/base/system/x": "gm_only" }),
            "a pointer naming no mergeable band",
        ),
        (
            serde_json::json!({ "/system/secret": "invisible" }),
            "not a tier",
        ),
        (serde_json::json!("gm_only"), "not an object"),
    ] {
        let mut doc = doc_with_engine(valid_wall_engine());
        let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
        base["property_overrides"] = policy;
        doc.base = Some(base);
        assert!(
            matches!(
                validate_engine_tree(&mut doc),
                Err(DataError::SchemaViolation { .. })
            ),
            "{what} must be rejected"
        );
    }
    let mut doc = doc_with_engine(valid_wall_engine());
    let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
    base["property_overrides"] =
        serde_json::json!({ "/system/secret": "gm_only", "/name": "owner_or_gm" });
    base["embedded"]["items"][0]["propertyOverrides"] =
        serde_json::json!({ "/engine/hp": "gm_only" });
    doc.base = Some(base.clone());
    validate_engine_tree(&mut doc).unwrap();
    assert_eq!(
        doc.base.unwrap(),
        base,
        "a well-formed policy round-trips verbatim"
    );
}

#[test]
fn validate_engine_tree_rejects_a_base_missing_a_band_key() {
    // Absent keys must be REJECTED, never coalesced by serde defaults: a
    // coalesced record reads as unchanged against a null band, which would
    // let a merge silently drop a template-deleted child.
    for missing in ["name", "engine", "system", "embedded"] {
        let mut doc = doc_with_engine(valid_wall_engine());
        let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
        base.as_object_mut().unwrap().remove(missing);
        doc.base = Some(base);
        assert!(
            matches!(
                validate_engine_tree(&mut doc),
                Err(DataError::SchemaViolation { .. })
            ),
            "a base missing '{missing}' must be rejected"
        );
    }
}

#[test]
fn validate_engine_tree_rejects_a_base_with_an_unknown_key() {
    let mut doc = doc_with_engine(valid_wall_engine());
    let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
    base.as_object_mut()
        .unwrap()
        .insert("smuggled".into(), serde_json::json!(1));
    doc.base = Some(base);
    assert!(matches!(
        validate_engine_tree(&mut doc),
        Err(DataError::SchemaViolation { .. })
    ));
}

#[test]
fn validate_engine_tree_rejects_a_base_with_wrong_typed_members() {
    let mut doc = doc_with_engine(valid_wall_engine());
    let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
    base["name"] = serde_json::json!(7);
    doc.base = Some(base.clone());
    assert!(
        matches!(
            validate_engine_tree(&mut doc),
            Err(DataError::SchemaViolation { .. })
        ),
        "a non-string non-null name must be rejected"
    );

    let mut doc = doc_with_engine(valid_wall_engine());
    base["name"] = serde_json::json!("Old");
    base["embedded"] = serde_json::json!([]);
    doc.base = Some(base);
    assert!(
        matches!(
            validate_engine_tree(&mut doc),
            Err(DataError::SchemaViolation { .. })
        ),
        "a non-object embedded map must be rejected"
    );
}

#[test]
fn validate_engine_tree_rejects_an_embedded_base_record_missing_a_key() {
    // The data-losing direction: an `EmbeddedBaseChild` record absent any of
    // its keys must not be admitted.
    for missing in ["sourceId", "name", "engine", "system", "embedded"] {
        let mut doc = doc_with_engine(valid_wall_engine());
        let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
        base["embedded"]["items"][0]
            .as_object_mut()
            .unwrap()
            .remove(missing);
        doc.base = Some(base);
        assert!(
            matches!(
                validate_engine_tree(&mut doc),
                Err(DataError::SchemaViolation { .. })
            ),
            "an embedded base record missing '{missing}' must be rejected"
        );
    }
}

#[test]
fn validate_engine_tree_rejects_a_non_string_source_id() {
    let mut doc = doc_with_engine(valid_wall_engine());
    let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
    base["embedded"]["items"][0]["sourceId"] = serde_json::json!(7);
    doc.base = Some(base);
    assert!(matches!(
        validate_engine_tree(&mut doc),
        Err(DataError::SchemaViolation { .. })
    ));
}

#[test]
fn validate_engine_tree_rejects_a_stale_schema_root_base_engine() {
    // The walk is not a pass-through: a snapshot whose root engine band is
    // invalid under the doc's CURRENT schema is rejected at ingest/rewrite.
    // (A legacy row carrying one still READS — validation is ingest-time
    // only; the read-path half is pinned in the repository tests.)
    let mut doc = doc_with_engine(valid_wall_engine());
    let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
    base["engine"] = serde_json::json!({ "seg": { "x1": "not-a-number" } });
    doc.base = Some(base);
    assert!(matches!(
        validate_engine_tree(&mut doc),
        Err(DataError::BadEngine(_))
    ));
}

#[test]
fn validate_engine_tree_normalizes_a_correlated_embedded_child_engine() {
    // An embedded base record correlated to a LIVE child (by `sourceId` ==
    // the child's `source.id`) is normalized under the CHILD's doc_type.
    let child_id = Uuid::from_u128(201);
    let template_child_id = Uuid::from_u128(202);
    let mut child = doc_with_system(serde_json::json!({}));
    child.id = child_id;
    child.source = Some(crate::data::document::Source {
        id: template_child_id,
        pack: None,
        version: 1,
    });
    child.engine = Some(serde_json::json!({
        "displayName": "Goblin",
        "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "faction": null,
        "conditions": [],
        "prototype": true
    }));
    let mut parent = doc_with_engine(valid_wall_engine());
    parent.embedded.insert("items".into(), vec![child]);
    parent.base = Some(serde_json::json!({
        "name": "Old",
        "engine": { "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 } },
        "system": {},
        "embedded": {
            "items": [{
                "sourceId": template_child_id.to_string(),
                "name": "Child",
                "engine": {
                    "displayName": "Goblin",
                    "visual": { "kind": "image", "asset": "a.png" },
                    "size": { "w": 1.0, "h": 1.0 },
                    "shape": "square",
                    "conditions": [],
                    "prototype": true
                    // "faction" intentionally omitted
                },
                "system": {},
                "embedded": {},
                "propertyOverrides": {}
            }]
        },
        "property_overrides": {},
        "owner_standing": "owner"
    }));
    validate_engine_tree(&mut parent).unwrap();
    let base = parent.base.unwrap();
    assert_eq!(
        base["embedded"]["items"][0]["engine"]["faction"],
        serde_json::Value::Null,
        "the correlated record's engine is normalized under the live child's doc_type"
    );
}

#[test]
fn validate_engine_tree_shape_checks_but_does_not_normalize_a_historical_record() {
    // No live counterpart for the record's `sourceId`: the record is
    // historical (the template child is gone), so it is shape-checked only —
    // its engine band must survive untouched even when no doc_type could
    // normalize it.
    let mut doc = doc_with_engine(valid_wall_engine());
    let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000ff");
    base["embedded"]["items"][0]["engine"] = serde_json::json!({ "not": "any-engine-shape" });
    doc.base = Some(base.clone());
    validate_engine_tree(&mut doc).unwrap();
    assert_eq!(doc.base.unwrap(), base);
}

#[test]
fn validate_engine_tree_recurses_into_nested_base_records() {
    let mut doc = doc_with_engine(valid_wall_engine());
    let mut base = valid_wall_base("00000000-0000-0000-0000-0000000000c1");
    base["embedded"]["items"][0]["embedded"] = serde_json::json!({
        "nested": [{
            "sourceId": "00000000-0000-0000-0000-0000000000c2",
            "name": "Grandchild"
            // remaining keys intentionally absent: must be rejected at depth
        }]
    });
    doc.base = Some(base);
    let err = validate_engine_tree(&mut doc).unwrap_err();
    match err {
        DataError::SchemaViolation { pointer, .. } => {
            assert_eq!(pointer, "/base/embedded/items/0/embedded/nested/0");
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

// --- validate_value_against_schema: accept/reject matrix ---
// `Schema` is already in scope via `use super::*` (top-of-file import).

fn obj_schema(props: serde_json::Value) -> Schema {
    // Build a Schema from a JSON literal (exercises the real deserialize path).
    serde_json::from_value(props).unwrap()
}

#[test]
fn scalar_type_match_and_mismatch() {
    let s: Schema = obj_schema(serde_json::json!({ "type": "number" }));
    assert!(validate_value_against_schema(&serde_json::json!(3), &s).is_ok());
    let err = validate_value_against_schema(&serde_json::json!("x"), &s).unwrap_err();
    assert_eq!(err.reason, "expected number, got string");
}

#[test]
fn nullable_accepts_null_and_non_nullable_rejects_null() {
    let n: Schema = obj_schema(serde_json::json!({ "type": "number", "nullable": true }));
    assert!(validate_value_against_schema(&serde_json::json!(null), &n).is_ok());
    let s: Schema = obj_schema(serde_json::json!({ "type": "number" }));
    let err = validate_value_against_schema(&serde_json::json!(null), &s).unwrap_err();
    assert_eq!(err.reason, "expected number, got null");
}

#[test]
fn null_type_requires_null() {
    let s: Schema = obj_schema(serde_json::json!({ "type": "null" }));
    assert!(validate_value_against_schema(&serde_json::json!(null), &s).is_ok());
    assert!(validate_value_against_schema(&serde_json::json!(0), &s).is_err());
}

#[test]
fn empty_schema_matches_any() {
    let any = Schema::default();
    assert!(
        validate_value_against_schema(&serde_json::json!({ "a": [1, "b", null] }), &any).is_ok()
    );
    assert!(validate_value_against_schema(&serde_json::json!(null), &any).is_ok());
}

#[test]
fn required_present_vs_missing() {
    let s: Schema = obj_schema(serde_json::json!({
        "type": "object", "required": ["kind"], "properties": { "kind": { "type": "string" } }
    }));
    assert!(validate_value_against_schema(&serde_json::json!({ "kind": "stat" }), &s).is_ok());
    let err = validate_value_against_schema(&serde_json::json!({}), &s).unwrap_err();
    assert_eq!(err.reason, "missing required key 'kind'");
    assert_eq!(err.pointer, "/kind");
}

#[test]
fn additional_properties_closed_by_default_rejects_unknown_key() {
    let s: Schema = obj_schema(serde_json::json!({
        "type": "object", "properties": { "a": { "type": "number" } }
    }));
    let err =
        validate_value_against_schema(&serde_json::json!({ "a": 1, "b": 2 }), &s).unwrap_err();
    assert_eq!(err.reason, "unknown key 'b' not permitted by schema");
    assert_eq!(err.pointer, "/b");
}

#[test]
fn additional_properties_subschema_accepts_open_map_and_rejects_wrong_type() {
    // A game system's author-keyed stat map: the engine knows none of the keys,
    // so every value is checked against one shared subschema instead.
    let s: Schema = obj_schema(serde_json::json!({
        "type": "object",
        "additionalProperties": { "type": "object", "required": ["kind"],
            "properties": { "kind": { "type": "string" } } }
    }));
    assert!(validate_value_against_schema(
        &serde_json::json!({ "str": { "kind": "ability" }, "dex": { "kind": "ability" } }),
        &s
    )
    .is_ok());
    let err = validate_value_against_schema(&serde_json::json!({ "str": { "kind": 5 } }), &s)
        .unwrap_err();
    assert_eq!(err.reason, "expected string, got number");
    assert_eq!(err.pointer, "/str/kind");
}

#[test]
fn additional_properties_true_permits_any_extra_key() {
    let s: Schema = obj_schema(serde_json::json!({
        "type": "object", "properties": { "a": { "type": "number" } },
        "additionalProperties": true
    }));
    assert!(validate_value_against_schema(&serde_json::json!({ "a": 1, "b": [1, 2] }), &s).is_ok());
}

#[test]
fn array_items_uniform_typing() {
    let s: Schema =
        obj_schema(serde_json::json!({ "type": "array", "items": { "type": "number" } }));
    assert!(validate_value_against_schema(&serde_json::json!([1, 2, 3]), &s).is_ok());
    let err = validate_value_against_schema(&serde_json::json!([1, "x"]), &s).unwrap_err();
    assert_eq!(err.reason, "expected number, got string");
    assert_eq!(err.pointer, "/1");
    // Not an array at all.
    assert!(validate_value_against_schema(&serde_json::json!({}), &s).is_err());
}

#[test]
fn array_without_items_accepts_mixed_elements() {
    let s: Schema = obj_schema(serde_json::json!({ "type": "array" }));
    assert!(validate_value_against_schema(&serde_json::json!([1, "x", null]), &s).is_ok());
}

// --- validate_system_schema_tree: subtree scoping, embedded, absent-ok ---

fn decl(doc_type: &str, pointer: &str, schema: serde_json::Value) -> SchemaDeclaration {
    SchemaDeclaration {
        module_id: "m".into(),
        version: "1".into(),
        schema_format: 1,
        doc_type: doc_type.into(),
        subtree_pointer: pointer.into(),
        schema: serde_json::from_value(schema).unwrap(),
    }
}

#[test]
fn tree_validator_rejects_a_violating_subtree_with_prefixed_pointer() {
    let doc = doc_with_system(serde_json::json!({ "stats": { "str": { "kind": 5 } } }));
    let schemas = vec![decl(
        "actor",
        "/system/stats",
        serde_json::json!({ "type": "object",
            "additionalProperties": { "type": "object",
                "properties": { "kind": { "type": "string" } } } }),
    )];
    let err = validate_system_schema_tree(&doc, &schemas).unwrap_err();
    match err {
        DataError::SchemaViolation { pointer, reason } => {
            assert_eq!(pointer, "/system/stats/str/kind");
            assert_eq!(reason, "expected string, got number");
        }
        other => panic!("expected SchemaViolation, got {other:?}"),
    }
}

#[test]
fn tree_validator_absent_subtree_is_ok() {
    let doc = doc_with_system(serde_json::json!({ "other": 1 }));
    let schemas = vec![decl(
        "actor",
        "/system/stats",
        serde_json::json!({ "type": "object" }),
    )];
    assert!(validate_system_schema_tree(&doc, &schemas).is_ok());
}

#[test]
fn tree_validator_unregistered_doc_type_passes() {
    let doc = doc_with_system(serde_json::json!({ "anything": true }));
    let schemas = vec![decl(
        "item",
        "/system/x",
        serde_json::json!({ "type": "number" }),
    )];
    assert!(validate_system_schema_tree(&doc, &schemas).is_ok());
}

#[test]
fn tree_validator_disjoint_subtrees_both_enforce() {
    let mut doc = doc_with_system(serde_json::json!({
        "stats": { "str": { "kind": "ability" } },
        "mechanics": { "version": "not-a-number" }
    }));
    doc.doc_type = "actor".into();
    let schemas = vec![
        decl(
            "actor",
            "/system/stats",
            serde_json::json!({ "type": "object",
            "additionalProperties": { "type": "object",
                "properties": { "kind": { "type": "string" } } } }),
        ),
        decl(
            "actor",
            "/system/mechanics",
            serde_json::json!({ "type": "object",
            "required": ["version"], "properties": { "version": { "type": "number" } } }),
        ),
    ];
    let err = validate_system_schema_tree(&doc, &schemas).unwrap_err();
    assert!(matches!(err, DataError::SchemaViolation { .. }));
}

#[test]
fn tree_validator_recurses_embedded_by_child_doc_type() {
    let mut parent = doc_with_system(serde_json::json!({}));
    parent.doc_type = "actor".into();
    let mut child = doc_with_system(serde_json::json!({ "power": { "cost": "free" } }));
    child.doc_type = "item".into();
    parent.embedded.insert("items".into(), vec![child]);
    let schemas = vec![decl(
        "item",
        "/system/power",
        serde_json::json!({ "type": "object", "properties": { "cost": { "type": "number" } } }),
    )];
    let err = validate_system_schema_tree(&parent, &schemas).unwrap_err();
    assert!(matches!(err, DataError::SchemaViolation { .. }));
}

// --- validate_containment: combat family placement rules ---

#[test]
fn validate_containment_rejects_a_parented_combat_and_an_unparented_combatant() {
    let mut combat = crate::data::document::tests::sample_doc();
    combat.doc_type = "combat".into();
    combat.engine = crate::data::document::tests::default_test_engine("combat");
    combat.parent_id = Some(uuid::Uuid::from_u128(5));
    assert!(validate_containment(&combat).is_err());
    combat.parent_id = None;
    assert!(validate_containment(&combat).is_ok());

    let mut combatant = crate::data::document::tests::sample_doc();
    combatant.doc_type = "combatant".into();
    combatant.engine = crate::data::document::tests::default_test_engine("combatant");
    combatant.parent_id = None;
    assert!(validate_containment(&combatant).is_err());
    combatant.parent_id = Some(uuid::Uuid::from_u128(5));
    assert!(validate_containment(&combatant).is_ok());
}

#[test]
fn validate_containment_requires_a_parent_for_combat_history_and_forbids_embedding_it() {
    let mut history = crate::data::document::tests::sample_doc();
    history.doc_type = "combat-history".into();
    history.engine = crate::data::document::tests::default_test_engine("combat-history");
    history.parent_id = None;
    assert!(validate_containment(&history).is_err());
    history.parent_id = Some(uuid::Uuid::from_u128(5));
    assert!(validate_containment(&history).is_ok());

    let mut child = crate::data::document::tests::sample_doc();
    child.doc_type = "combat-history".into();
    child.engine = crate::data::document::tests::default_test_engine("combat-history");
    child.parent_id = Some(uuid::Uuid::from_u128(5));
    let mut parent = crate::data::document::tests::sample_doc();
    parent.embedded.insert("combat-history".into(), vec![child]);
    assert!(validate_containment(&parent).is_err());
}

#[test]
fn validate_containment_rejects_combat_and_combatant_as_embedded_children() {
    for child_type in ["combat", "combatant"] {
        let mut child = crate::data::document::tests::sample_doc();
        child.doc_type = child_type.into();
        child.engine = crate::data::document::tests::default_test_engine(child_type);
        child.parent_id = if child_type == "combatant" {
            Some(uuid::Uuid::from_u128(5))
        } else {
            None
        };
        let mut parent = crate::data::document::tests::sample_doc();
        parent.embedded.insert(child_type.into(), vec![child]);
        assert!(
            validate_containment(&parent).is_err(),
            "{child_type} must not embed"
        );
    }
}

#[test]
fn tree_validator_grandchild_violation_rejects() {
    let mut parent = doc_with_system(serde_json::json!({}));
    parent.doc_type = "actor".into();
    let mut child = doc_with_system(serde_json::json!({}));
    child.doc_type = "container".into();
    let mut gc = doc_with_system(serde_json::json!({ "power": { "cost": "free" } }));
    gc.doc_type = "item".into();
    child.embedded.insert("nested".into(), vec![gc]);
    parent.embedded.insert("items".into(), vec![child]);
    let schemas = vec![decl(
        "item",
        "/system/power",
        serde_json::json!({ "type": "object", "properties": { "cost": { "type": "number" } } }),
    )];
    assert!(matches!(
        validate_system_schema_tree(&parent, &schemas),
        Err(DataError::SchemaViolation { .. })
    ));
}

#[test]
fn validate_containment_forbids_embedding_an_asset_folder() {
    let mut actor = crate::data::document::tests::sample_doc();
    let mut folder = crate::data::document::tests::sample_doc();
    folder.doc_type = "asset_folder".into();
    folder.engine = Some(serde_json::json!({ "sort": 0 }));
    actor.embedded.insert("stuff".into(), vec![folder]);
    assert!(validate_containment(&actor).is_err());
}
