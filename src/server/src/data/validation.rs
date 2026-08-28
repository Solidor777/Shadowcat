// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::data::document::{
    AdditionalProperties, Document, Schema, SchemaDeclaration, SchemaType,
};
use crate::data::engine;
use crate::data::DataError;

/// Maximum serialized size of EACH opaque body block (`system`, `engine`,
/// `base`) independently. Region/drawing point arrays make `engine`
/// size-unbounded without this cap; the name is kept as
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
    /// JSON pointer (relative to the validated root) of the offending node.
    pub pointer: String,
    /// Shape-only description; never echoes the value's content.
    pub reason: String,
}

/// The JSON type name of a value, for structural error phrasing.
///
/// # Examples
///
/// ```text
/// json_type_name(&json!(3)) == "number"
/// ```
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

/// Shape-only match of a JSON value against a schema type-tree node.
/// NEVER inspects a value's magnitude/content: scalars
/// match on JSON type alone. `additionalProperties` defaults to closed.
pub fn validate_value_against_schema(
    value: &serde_json::Value,
    schema: &Schema,
) -> Result<(), SchemaMismatch> {
    check_value(value, schema, String::new())
}

/// Recursive worker for `validate_value_against_schema`; `at` accumulates the
/// JSON pointer reported on mismatch.
///
/// # Examples
///
/// ```text
/// check_value(&json!({}), &schema, String::new()) // pointer "" = the root
/// ```
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
                // which defaults to closed when absent.
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

/// Validate the POST-IMAGE `system` band against the world's registered
/// structural schemas, recursing embedded descendants — each
/// looked up by its OWN `doc_type`. READ-ONLY: unlike `validate_engine_tree`,
/// there is no normalization; tier-2 only accepts/rejects and must not reshape
/// the opaque `system` body. A subtree registered but absent in this document is
/// NOT a violation (registering a schema governs shape-when-present, never
/// compels presence). `subtree_pointer` is a strict `/system/…` descendant
/// (guaranteed at set-time by `validate_schema_declarations`), so the leading
/// `/system` is stripped and the remainder resolved within `doc.system`.
pub fn validate_system_schema_tree(
    doc: &Document,
    schemas: &[SchemaDeclaration],
) -> Result<(), DataError> {
    for decl in schemas {
        if decl.doc_type != doc.doc_type {
            continue;
        }
        // Strict `/system/…` descendant → strip the `/system` prefix and resolve
        // the remainder (`/stats`, `/mechanics/version`, …) inside `doc.system`.
        let rel = &decl.subtree_pointer["/system".len()..];
        let Some(subtree) = doc.system.pointer(rel) else {
            continue; // absent subtree: not a violation
        };
        if let Err(m) = validate_value_against_schema(subtree, &decl.schema) {
            return Err(DataError::SchemaViolation {
                pointer: format!("{}{}", decl.subtree_pointer, m.pointer),
                reason: m.reason,
            });
        }
    }
    for children in doc.embedded.values() {
        for child in children {
            validate_system_schema_tree(child, schemas)?;
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

/// Structural gate for one `FieldChange`: a well-formed path, plus the rule that a
/// REMOVAL carries no value.
///
/// `remove: true` deletes the key at `path` and `new` is unused (conventionally
/// `Null`), so `remove: true` with a non-null `new` is a wire shape with no legitimate
/// meaning. Rejecting it is defence in depth for a real divergence class: `new` is
/// checked by NEITHER the OCC pre-image comparison (which reads `old`) NOR
/// `required_cap_for_path`, so any consumer that mirrors a change by unconditionally
/// setting `new` — instead of branching on `remove` as `apply_intent` Phase 2 does —
/// lands an attacker-chosen value while the store lands absence. The derived scene ECS
/// had exactly that bug; `command::apply_field_change` is now the single store-equal
/// rule, called by every authoritative path and every mirror. Denying the shape at
/// ingress means no future mirror can be forked this way even if it re-introduces the
/// same mistake.
pub fn validate_field_change(ch: &crate::data::command::FieldChange) -> Result<(), DataError> {
    validate_field_path(&ch.path)?;
    if ch.remove && !ch.new.is_null() {
        return Err(DataError::OpFailed(format!(
            "a removal at {} must not carry a `new` value",
            ch.path
        )));
    }
    Ok(())
}

/// Reject a `property_overrides` key that either is not a well-formed
/// non-empty JSON pointer, or names something redaction cannot classify.
///
/// A well-formed pointer must start with `/` and must NOT end with `/`. A
/// trailing slash (e.g. `/engine/`) fails to exact-match its intended target
/// AND fails to match as a valid nested pointer under it, so the override
/// silently no-ops — a fail-OPEN footgun where a GM/author believes a
/// property is hidden but `can_see` never consults the malformed key.
///
/// A well-formed pointer is then checked against
/// `crate::data::permission::redaction_target`: redaction operates on
/// content bands (`name`/`engine`/`system`/`base`) only, never on the
/// structural envelope (`id`, `owner`, `permissions` itself, etc). A pointer
/// `redaction_target` cannot classify is refused here so no stored override
/// can later ask egress to remove a field it must not touch.
///
/// Recurses into every embedded descendant's own `property_overrides`,
/// mirroring `validate_system_size`'s embedded-tree walk.
pub fn validate_property_overrides(doc: &Document) -> Result<(), DataError> {
    for key in doc.permissions.property_overrides.keys() {
        if key.is_empty() || !key.starts_with('/') || key.ends_with('/') {
            return Err(DataError::BadPath(key.clone()));
        }
        if crate::data::permission::redaction_target(key).is_none() {
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

/// Placement rules for the combat family that need no database: a `combat`
/// is never parented and never embedded; a `combatant` is always parented
/// (its parent must be a `combat`, checked at the persistence chokepoint
/// where the parent can be loaded) and never embedded. Recurses into every
/// embedded descendant.
pub fn validate_containment(doc: &Document) -> Result<(), DataError> {
    match doc.doc_type.as_str() {
        t if t == engine::COMBAT_DOC_TYPE && doc.parent_id.is_some() => {
            return Err(DataError::OpFailed(
                "a combat document cannot have a parent".into(),
            ));
        }
        t if t == engine::COMBATANT_DOC_TYPE && doc.parent_id.is_none() => {
            return Err(DataError::OpFailed(
                "a combatant document requires a parent combat".into(),
            ));
        }
        _ => {}
    }
    for children in doc.embedded.values() {
        for child in children {
            if child.doc_type == engine::COMBAT_DOC_TYPE
                || child.doc_type == engine::COMBATANT_DOC_TYPE
            {
                return Err(DataError::OpFailed(format!(
                    "a '{}' document cannot be embedded",
                    child.doc_type
                )));
            }
            validate_containment(child)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
