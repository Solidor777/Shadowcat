use crate::data::document::{AdditionalProperties, Document, Schema, SchemaType};
use crate::data::engine;
use crate::data::DataError;

/// Maximum serialized size of EACH opaque body block (`system`, `engine`,
/// `base`) independently. Region/drawing point arrays make `engine`
/// size-unbounded without this cap (spec S6); the name is kept as
/// `MAX_SYSTEM_BYTES` since it is referenced by that name across the
/// codebase, but it now bounds every block, not just `system`.
pub const MAX_SYSTEM_BYTES: usize = 256 * 1024;

/// Reject a document — and every embedded descendant — whose opaque `system`
/// body, or (when present) typed `engine` body, or (when present) opaque
/// `base` snapshot, exceeds the per-block size cap. Embedded children are
/// stored inline in the parent JSON, so each body is bounded independently;
/// the recursion mirrors `embedded`'s finite stored depth (a document cannot
/// embed itself).
pub fn validate_system_size(doc: &Document) -> Result<(), DataError> {
    let bytes = serde_json::to_vec(&doc.system)?.len();
    if bytes > MAX_SYSTEM_BYTES {
        return Err(DataError::TooLarge(bytes));
    }
    if let Some(eng) = &doc.engine {
        let eng_bytes = serde_json::to_vec(eng)?.len();
        if eng_bytes > MAX_SYSTEM_BYTES {
            return Err(DataError::TooLarge(eng_bytes));
        }
    }
    if let Some(base) = &doc.base {
        let base_bytes = serde_json::to_vec(base)?.len();
        if base_bytes > MAX_SYSTEM_BYTES {
            return Err(DataError::TooLarge(base_bytes));
        }
    }
    for children in doc.embedded.values() {
        for child in children {
            validate_system_size(child)?;
        }
    }
    Ok(())
}

/// Validate the POST-IMAGE `engine` band against `doc.doc_type`'s typed
/// struct (`engine::validate_engine`), recursing into embedded descendants,
/// and — on success — REPLACE `doc.engine` (and each descendant's) with the
/// re-serialized validated struct rather than the raw submitted JSON. This
/// is the single chokepoint every persistence path (Create; Update
/// post-image; embedded mutation) calls before storing a document.
///
/// For `Update`, `apply_intent`'s Phase 2 additionally re-derives every
/// `/engine`(/*) `FieldChange.new` from this SAME normalized `doc` before the
/// `world_events` INSERT, so the normalized form reaches not just the
/// persisted row but also the broadcast delta and the permanent event log
/// (and therefore every future `events_since` replay) — never the raw
/// client-submitted JSON. `/system`-prefixed changes are untouched by that
/// step; only the structurally-typed engine band goes through this function.
///
/// Re-serializing (not pass-through) compensates for two ingress gaps:
/// (a) internally-tagged enums (`TokenVisual`/`RenderVisual`/`AnimatedSource`)
/// cannot carry `#[serde(deny_unknown_fields)]` (a serde limitation), so an
/// unknown key smuggled into one of those sub-objects survives structural
/// validation but is structurally dropped by this deserialize-then-reserialize
/// round trip — Rust never retains a field it didn't deserialize; (b) an
/// ingress-absent optional field (e.g. `ActorEngine.faction`) deserializes to
/// `None`, and the persisted/broadcast form must store that as an explicit
/// `null` to match the client's `T | null` contract, not silently omit the key.
///
/// `doc.base` is NEVER walked here: it is a historical opaque snapshot that
/// may hold an engine shape invalid under the doc's CURRENT schema, and must
/// still store as-is (size-capped separately by `validate_system_size`).
pub fn validate_engine_tree(doc: &mut Document) -> Result<(), DataError> {
    doc.engine = engine::normalize_engine_opt(&doc.doc_type, doc.engine.as_ref())?;
    for children in doc.embedded.values_mut() {
        for child in children {
            validate_engine_tree(child)?;
        }
    }
    Ok(())
}

/// A structural mismatch: the JSON pointer (relative to the validated value's
/// root) of the offending location plus a shape-only reason. Never carries a
/// value's content.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaMismatch {
    pub pointer: String,
    pub reason: String,
}

/// The JSON type name of a value, for structural error phrasing.
fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// The schema type name, for structural error phrasing.
fn schema_type_label(t: SchemaType) -> &'static str {
    match t {
        SchemaType::Object => "object",
        SchemaType::Array => "array",
        SchemaType::String => "string",
        SchemaType::Number => "number",
        SchemaType::Boolean => "boolean",
        SchemaType::Null => "null",
    }
}

/// RFC-6901 reference-token escaping: `~` -> `~0`, `/` -> `~1`. Keeps a member
/// key with a slash from forging a spurious pointer segment.
fn escape_token(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Shape-only match of a JSON value against a schema type-tree node (M13f
/// tier-2). NEVER inspects a value's magnitude/content (invariant 6): scalars
/// match on JSON type alone. `additionalProperties` defaults to closed (F2).
pub fn validate_value_against_schema(
    value: &serde_json::Value,
    schema: &Schema,
) -> Result<(), SchemaMismatch> {
    check_value(value, schema, String::new())
}

fn check_value(
    value: &serde_json::Value,
    schema: &Schema,
    at: String,
) -> Result<(), SchemaMismatch> {
    // A typeless node (`{}`) matches any JSON value.
    let Some(ty) = schema.ty else {
        return Ok(());
    };
    // `nullable: true` widens exactly this node to also accept JSON null. The
    // `null` type accepts null inherently.
    if value.is_null() {
        if ty == SchemaType::Null || schema.nullable == Some(true) {
            return Ok(());
        }
        return Err(SchemaMismatch {
            pointer: at,
            reason: format!("expected {}, got null", schema_type_label(ty)),
        });
    }
    match ty {
        SchemaType::Null => Err(SchemaMismatch {
            pointer: at,
            reason: format!("expected null, got {}", json_type_name(value)),
        }),
        SchemaType::Boolean if !value.is_boolean() => Err(SchemaMismatch {
            pointer: at,
            reason: format!("expected boolean, got {}", json_type_name(value)),
        }),
        SchemaType::Number if !value.is_number() => Err(SchemaMismatch {
            pointer: at,
            reason: format!("expected number, got {}", json_type_name(value)),
        }),
        SchemaType::String if !value.is_string() => Err(SchemaMismatch {
            pointer: at,
            reason: format!("expected string, got {}", json_type_name(value)),
        }),
        SchemaType::Boolean | SchemaType::Number | SchemaType::String => Ok(()),
        SchemaType::Array => {
            let Some(arr) = value.as_array() else {
                return Err(SchemaMismatch {
                    pointer: at,
                    reason: format!("expected array, got {}", json_type_name(value)),
                });
            };
            if let Some(items) = &schema.items {
                for (i, el) in arr.iter().enumerate() {
                    check_value(el, items, format!("{at}/{i}"))?;
                }
            }
            Ok(())
        }
        SchemaType::Object => {
            let Some(obj) = value.as_object() else {
                return Err(SchemaMismatch {
                    pointer: at,
                    reason: format!("expected object, got {}", json_type_name(value)),
                });
            };
            if let Some(required) = &schema.required {
                for key in required {
                    if !obj.contains_key(key) {
                        return Err(SchemaMismatch {
                            pointer: format!("{at}/{}", escape_token(key)),
                            reason: format!("missing required key '{key}'"),
                        });
                    }
                }
            }
            for (key, val) in obj {
                let child_ptr = format!("{at}/{}", escape_token(key));
                if let Some(props) = &schema.properties {
                    if let Some(sub) = props.get(key) {
                        check_value(val, sub, child_ptr)?;
                        continue;
                    }
                }
                // Key not in `properties`: governed by additionalProperties,
                // which defaults to closed (F2) when absent.
                match &schema.additional_properties {
                    None | Some(AdditionalProperties::Bool(false)) => {
                        return Err(SchemaMismatch {
                            pointer: child_ptr,
                            reason: format!("unknown key '{key}' not permitted by schema"),
                        });
                    }
                    Some(AdditionalProperties::Bool(true)) => {}
                    Some(AdditionalProperties::Schema(sub)) => {
                        check_value(val, sub, child_ptr)?;
                    }
                }
            }
            Ok(())
        }
    }
}

/// A valid JSON pointer is empty or a sequence of "/"-prefixed tokens.
pub fn validate_field_path(path: &str) -> Result<(), DataError> {
    if path.is_empty() {
        return Ok(());
    }
    if !path.starts_with('/') {
        return Err(DataError::BadPath(path.to_string()));
    }
    Ok(())
}

/// Reject a `property_overrides` key that is not a well-formed non-empty JSON
/// pointer: it must start with `/` and must NOT end with `/`. A trailing
/// slash (e.g. `/engine/`) fails to exact-match its intended target AND fails
/// to match as a valid nested pointer under it, so the override silently
/// no-ops — a fail-OPEN footgun where a GM/author believes a property is
/// hidden but `can_see` never consults the malformed key. Recurses into every
/// embedded descendant's own `property_overrides`, mirroring
/// `validate_system_size`'s embedded-tree walk.
pub fn validate_property_overrides(doc: &Document) -> Result<(), DataError> {
    for key in doc.permissions.property_overrides.keys() {
        if key.is_empty() || !key.starts_with('/') || key.ends_with('/') {
            return Err(DataError::BadPath(key.clone()));
        }
    }
    for children in doc.embedded.values() {
        for child in children {
            validate_property_overrides(child)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
        let child =
            doc_with_system(serde_json::json!({ "blob": "x".repeat(MAX_SYSTEM_BYTES + 1) }));
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
        let child =
            doc_with_engine(serde_json::json!({ "blob": "x".repeat(MAX_SYSTEM_BYTES + 1) }));
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

    // --- base: independent size cap + engine-validation exemption ---

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

    #[test]
    fn base_holding_stale_engine_is_exempt_from_engine_validation() {
        // base is a historical snapshot that may predate the current engine schema; it must
        // store even when it carries an engine shape that is invalid for this doc_type.
        let mut doc = doc_with_engine(valid_wall_engine());
        doc.base = Some(serde_json::json!({
            "name": "Old", "engine": { "seg": { "x1": "not-a-number" } },
            "system": {}, "embedded": {}
        }));
        assert!(
            validate_engine_tree(&mut doc).is_ok(),
            "base must not be walked by validate_engine_tree"
        );
        // And the stale base survives untouched.
        assert_eq!(
            doc.base.unwrap()["engine"]["seg"]["x1"],
            serde_json::json!("not-a-number")
        );
    }

    // --- validate_value_against_schema: accept/reject matrix (M13f tier-2) ---
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
            validate_value_against_schema(&serde_json::json!({ "a": [1, "b", null] }), &any)
                .is_ok()
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
        // The Nightfox open user-keyed stat map.
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
        assert!(
            validate_value_against_schema(&serde_json::json!({ "a": 1, "b": [1, 2] }), &s).is_ok()
        );
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
}
