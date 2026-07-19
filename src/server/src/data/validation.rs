use crate::data::document::Document;
use crate::data::engine;
use crate::data::DataError;

/// Maximum serialized size of EACH opaque body block (`system`, `engine`)
/// independently. Region/drawing point arrays make `engine` size-unbounded
/// without this cap (spec S6); the name is kept as `MAX_SYSTEM_BYTES` since
/// it is referenced by that name across the codebase, but it now bounds
/// every block, not just `system`.
pub const MAX_SYSTEM_BYTES: usize = 256 * 1024;

/// Reject a document — and every embedded descendant — whose opaque `system`
/// body, or (when present) typed `engine` body, exceeds the per-block size
/// cap. Embedded children are stored inline in the parent JSON, so each body
/// is bounded independently; the recursion mirrors `embedded`'s finite stored
/// depth (a document cannot embed itself).
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
pub fn validate_engine_tree(doc: &mut Document) -> Result<(), DataError> {
    doc.engine = engine::normalize_engine_opt(&doc.doc_type, doc.engine.as_ref())?;
    for children in doc.embedded.values_mut() {
        for child in children {
            validate_engine_tree(child)?;
        }
    }
    Ok(())
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
}
