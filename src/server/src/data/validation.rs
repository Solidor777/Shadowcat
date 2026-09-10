// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::data::document::{
    AdditionalProperties, Document, OwnerStanding, Schema, SchemaDeclaration, SchemaType,
    Visibility,
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
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::validation::validate_system_size;
///
/// fn doc(system: serde_json::Value) -> Document {
///     Document {
///         id: uuid::Uuid::new_v4(),
///         scope: Scope::Compendium { pack: "core".into() },
///         doc_type: "note".into(),
///         schema_version: 1,
///         name: None,
///         source: None,
///         base: None,
///         owner: None,
///         permissions: PermissionSet::default(),
///         embedded: Default::default(),
///         parent_id: None,
///         engine: None,
///         system,
///         created_at: 0,
///         updated_at: 0,
///     }
/// }
///
/// assert!(validate_system_size(&doc(serde_json::json!({ "hp": 10 }))).is_ok());
///
/// let oversized = serde_json::json!({ "text": "x".repeat(300_000) });
/// assert!(validate_system_size(&doc(oversized)).is_err());
/// ```
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
/// `doc.base` IS walked here: it is server-owned (derived at Create by
/// `merge::bands::derive_create_base`, refreshed whole-band by server merge
/// writes, never client-writable), so it is shape-checked as a
/// `merge::bands::MergeBase` recursively and each `engine` band inside it is
/// normalized via the SAME `normalize_engine_opt` — the root's under the
/// document's own `doc_type`, an embedded base child's under the `doc_type`
/// of the LIVE embedded child its `sourceId` correlates to (mirroring
/// `snapshot_base`'s keying; a record with no live counterpart is
/// historical, so it is shape-checked only, never normalized). A legacy row
/// predating this walk still READS — validation is ingest-time only, as
/// everywhere else — and is re-validated only when rewritten.
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::validation::validate_engine_tree;
///
/// let mut doc = Document {
///     id: uuid::Uuid::new_v4(),
///     scope: Scope::Compendium { pack: "core".into() },
///     doc_type: "asset_folder".into(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::json!({ "sort": 0 })),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// assert!(validate_engine_tree(&mut doc).is_ok());
/// assert_eq!(doc.engine, Some(serde_json::json!({ "sort": 0 })));
/// ```
pub fn validate_engine_tree(doc: &mut Document) -> Result<(), DataError> {
    doc.engine = engine::normalize_engine_opt(&doc.doc_type, doc.engine.as_ref())?;
    if let Some(mut base) = doc.base.take() {
        // `doc.base` is detached for the walk so the live tree can be read
        // for embedded-child correlation while the snapshot is mutated.
        validate_base_node(&mut base, "/base", Some(doc), false)?;
        doc.base = Some(base);
    }
    for children in doc.embedded.values_mut() {
        for child in children {
            validate_engine_tree(child)?;
        }
    }
    Ok(())
}

/// The value the store would hold at `path` had `pre_image` been written
/// there — the OCC comparand for an engine-band pre-image. `whole` is the
/// serialized stored document (`serde_json::to_value` of the current row).
///
/// A stored engine band is `normalize_engine_opt`'s OUTPUT, not the client's
/// input: an absent `Option` field is re-serialized as an explicit `null`, so
/// a client pre-image built from its own optimistic view (carrying only the
/// keys it set) differs from the stored value by key count alone even when
/// it is faithful. `SqliteRepository::apply_intent`'s Phase-1 OCC check
/// therefore reads the pre-image through the SAME normalizer the stored value
/// came from, by splicing it into a copy of the stored document at `path`
/// and running `validate_engine_tree` on the result — the very function
/// Phase 2 runs to produce what gets stored, which also normalizes an
/// embedded child's band under the CHILD's `doc_type` (a bare
/// `normalize_engine_opt(root doc_type, …)` would misread
/// `/embedded/actor/0/engine`). The value at `path` is then re-extracted.
///
/// Returns `None` — the caller then falls back to the raw comparison, which
/// is never weaker — when `path` is not an engine-band path
/// (`permission::targets_engine_band`: the `system` band has no normalizer
/// and an explicit `null` there is user data, so absent-vs-null must stay a
/// real disagreement), or when any step fails (the spliced pre-image does
/// not deserialize as the typed engine, the splice lands on an ungrowable
/// array index, the document re-parse fails). Only the pointer's own value
/// is compared; envelope fields the round trip re-serializes (`permissions`
/// defaults and the like) never reach the comparison because `path` lies
/// inside a band. What the normalizer cannot represent stops being fatal —
/// absent-vs-null, and a key an internally-tagged enum silently drops — and
/// nothing else: a pre-image omitting or disagreeing on a key the store holds
/// with a real value normalizes to a value that still differs.
pub(crate) fn normalized_engine_pre_image(
    whole: &serde_json::Value,
    path: &str,
    pre_image: &serde_json::Value,
) -> Option<serde_json::Value> {
    if !crate::data::permission::targets_engine_band(path) {
        return None;
    }
    let mut probe = whole.clone();
    crate::data::command::set_pointer(&mut probe, path, pre_image.clone()).ok()?;
    let mut probe: Document = serde_json::from_value(probe).ok()?;
    validate_engine_tree(&mut probe).ok()?;
    let probe = serde_json::to_value(probe).ok()?;
    Some(
        probe
            .pointer(path)
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
}

/// The band keys every `MergeBase`-shaped node must carry exactly; an
/// embedded child record additionally carries `sourceId`, the root
/// additionally `BASE_STANDING_KEY`. The recorded policy key is required
/// too, under the node's own spelling (`base_policy_key`).
const BASE_BAND_KEYS: [&str; 4] = ["name", "engine", "system", "embedded"];

/// The key the ROOT of a stored base records the instance owner's standing
/// on the template under (`merge::bands::StoredBase::owner_standing`).
/// Required at the root — an absent standing would leave egress with no rule
/// for the whole snapshot, the fail-open direction — and rejected on a
/// record, where the root's standing applies.
const BASE_STANDING_KEY: &str = "owner_standing";

/// The key a `MergeBase`-shaped node records its content-band policy under:
/// `MergeBase` spells it `property_overrides`, an `EmbeddedBaseChild` record
/// `propertyOverrides` (the record's other keys are camelCase). Required at
/// ingest — an absent map would read as "nothing hidden", the fail-open
/// direction for the egress redaction that reads it.
fn base_policy_key(is_child: bool) -> &'static str {
    if is_child {
        "propertyOverrides"
    } else {
        "property_overrides"
    }
}

/// Shape-check one `MergeBase`-shaped node (`is_child` selects the
/// `EmbeddedBaseChild` key set): an object carrying every required key and
/// no others, `name` a string or null, `embedded` an object of arrays, the
/// recorded policy an object whose keys name a mergeable band
/// (`writes_a_content_band` — the only pointers `snapshot_base` records, and
/// the only ones the egress reader `permission`'s `base_policy` acts on) and
/// whose values parse as `Visibility`, for the root an `owner_standing` that
/// parses as `OwnerStanding`, and — for a child record — `sourceId` a
/// string. Reads nothing but shape.
fn check_base_node_shape(
    node: &serde_json::Value,
    pointer: &str,
    is_child: bool,
) -> Result<(), DataError> {
    let shape_err = |reason: String| DataError::SchemaViolation {
        pointer: pointer.to_string(),
        reason,
    };
    let Some(obj) = node.as_object() else {
        return Err(shape_err(format!(
            "expected object, got {}",
            json_type_name(node)
        )));
    };
    let policy_key = base_policy_key(is_child);
    for key in BASE_BAND_KEYS.iter().chain(std::iter::once(&policy_key)) {
        if !obj.contains_key(*key) {
            return Err(shape_err(format!("missing required key '{key}'")));
        }
    }
    if is_child && !obj.contains_key("sourceId") {
        return Err(shape_err("missing required key 'sourceId'".to_string()));
    }
    if !is_child && !obj.contains_key(BASE_STANDING_KEY) {
        return Err(shape_err(format!(
            "missing required key '{BASE_STANDING_KEY}'"
        )));
    }
    for key in obj.keys() {
        if BASE_BAND_KEYS.contains(&key.as_str())
            || key == policy_key
            || (is_child && key == "sourceId")
            || (!is_child && key == BASE_STANDING_KEY)
        {
            continue;
        }
        return Err(shape_err(format!(
            "unknown key '{key}' not permitted in a merge base"
        )));
    }
    let Some(policy) = obj[policy_key].as_object() else {
        return Err(shape_err(format!(
            "expected object at '{policy_key}', got {}",
            json_type_name(&obj[policy_key])
        )));
    };
    for (p, tier) in policy {
        if !crate::data::permission::writes_a_content_band(p) {
            return Err(shape_err(format!(
                "pointer '{p}' in '{policy_key}' names no mergeable band"
            )));
        }
        if serde_json::from_value::<Visibility>(tier.clone()).is_err() {
            return Err(shape_err(format!(
                "expected a visibility tier at '{policy_key}/{}', got {}",
                escape_token(p),
                json_type_name(tier)
            )));
        }
    }
    if !is_child && serde_json::from_value::<OwnerStanding>(obj[BASE_STANDING_KEY].clone()).is_err()
    {
        return Err(shape_err(format!(
            "expected an owner standing at '{BASE_STANDING_KEY}', got {}",
            json_type_name(&obj[BASE_STANDING_KEY])
        )));
    }
    let name = &obj["name"];
    if !(name.is_string() || name.is_null()) {
        return Err(shape_err(format!(
            "expected string or null at 'name', got {}",
            json_type_name(name)
        )));
    }
    if is_child && !obj["sourceId"].is_string() {
        return Err(shape_err(format!(
            "expected string at 'sourceId', got {}",
            json_type_name(&obj["sourceId"])
        )));
    }
    let Some(embedded) = obj["embedded"].as_object() else {
        return Err(shape_err(format!(
            "expected object at 'embedded', got {}",
            json_type_name(&obj["embedded"])
        )));
    };
    for (coll, records) in embedded {
        if !records.is_array() {
            return Err(shape_err(format!(
                "expected array at 'embedded/{}', got {}",
                escape_token(coll),
                json_type_name(records)
            )));
        }
    }
    Ok(())
}

/// The live embedded child a base record correlates to: the child whose
/// `source.id` (== its template child's id) matches `source_id`, falling
/// back to the child's own id — the same keying `snapshot_base` writes.
fn live_base_counterpart<'a>(
    live: &'a Document,
    collection: &str,
    source_id: &str,
) -> Option<&'a Document> {
    let kids = live.embedded.get(collection)?;
    kids.iter()
        .find(|k| {
            k.source
                .as_ref()
                .is_some_and(|s| s.id.to_string() == source_id)
        })
        .or_else(|| kids.iter().find(|k| k.id.to_string() == source_id))
}

/// Shape-check `node` (a `MergeBase` root when `is_child` is false, an
/// `EmbeddedBaseChild` record otherwise) and normalize its `engine` band in
/// place via `normalize_engine_opt` under `live`'s `doc_type`, recursing
/// into the `embedded` records with each record correlated to its live
/// counterpart (`live_base_counterpart`). `live: None` means the record is
/// historical (no live counterpart): shape-check only, no normalization.
fn validate_base_node(
    node: &mut serde_json::Value,
    pointer: &str,
    live: Option<&Document>,
    is_child: bool,
) -> Result<(), DataError> {
    check_base_node_shape(node, pointer, is_child)?;
    if let Some(live_doc) = live {
        let engine_band = &node["engine"];
        let engine_ref = (!engine_band.is_null()).then_some(engine_band);
        let normalized = engine::normalize_engine_opt(&live_doc.doc_type, engine_ref)?;
        node["engine"] = normalized.unwrap_or(serde_json::Value::Null);
    }
    let embedded = node["embedded"]
        .as_object_mut()
        .expect("check_base_node_shape requires 'embedded' to be an object");
    for (coll, records) in embedded {
        let records = records
            .as_array_mut()
            .expect("check_base_node_shape requires embedded values to be arrays");
        for (i, record) in records.iter_mut().enumerate() {
            let child_pointer = format!("{pointer}/embedded/{}/{}", escape_token(coll), i);
            let child_live = live
                .and_then(|l| live_base_counterpart(l, coll, record.get("sourceId")?.as_str()?));
            validate_base_node(record, &child_pointer, child_live, true)?;
        }
    }
    Ok(())
}

/// A structural mismatch: the JSON pointer (relative to the validated value's
/// root) of the offending location plus a shape-only reason. Never carries a
/// value's content.
///
/// # Examples
///
/// ```
/// use shadowcat::data::validation::SchemaMismatch;
///
/// let mismatch = SchemaMismatch {
///     pointer: "/hp".into(),
///     reason: "expected number, got string".into(),
/// };
/// assert_eq!(mismatch.pointer, "/hp");
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Schema, SchemaType};
/// use shadowcat::data::validation::validate_value_against_schema;
///
/// let schema = Schema { ty: Some(SchemaType::Number), ..Default::default() };
/// assert!(validate_value_against_schema(&serde_json::json!(3), &schema).is_ok());
///
/// let mismatch = validate_value_against_schema(&serde_json::json!("nope"), &schema).unwrap_err();
/// assert_eq!(mismatch.reason, "expected number, got string");
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, PermissionSet, Schema, SchemaDeclaration, SchemaType, Scope};
/// use shadowcat::data::validation::validate_system_schema_tree;
///
/// let doc = Document {
///     id: uuid::Uuid::new_v4(),
///     scope: Scope::Compendium { pack: "core".into() },
///     doc_type: "actor".into(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: None,
///     system: serde_json::json!({ "hp": 10 }),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let decl = SchemaDeclaration {
///     module_id: "example-module".into(),
///     version: "1.0.0".into(),
///     schema_format: 1,
///     doc_type: "actor".into(),
///     subtree_pointer: "/system/hp".into(),
///     schema: Schema { ty: Some(SchemaType::Number), ..Default::default() },
/// };
/// assert!(validate_system_schema_tree(&doc, &[decl]).is_ok());
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::data::validation::validate_field_path;
///
/// assert!(validate_field_path("/system/hp").is_ok());
/// assert!(validate_field_path("").is_ok()); // the root pointer
/// assert!(validate_field_path("system/hp").is_err()); // missing leading `/`
/// ```
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
///
/// # Examples
///
/// ```
/// use serde_json::json;
/// use shadowcat::data::command::FieldChange;
/// use shadowcat::data::validation::validate_field_change;
///
/// let removal = FieldChange { path: "/system/hp".into(), old: json!(10), new: json!(null), remove: true };
/// assert!(validate_field_change(&removal).is_ok());
///
/// let malformed = FieldChange { path: "/system/hp".into(), old: json!(10), new: json!(7), remove: true };
/// assert!(validate_field_change(&malformed).is_err()); // a removal must not carry `new`
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, PermissionSet, Scope, Visibility};
/// use shadowcat::data::validation::validate_property_overrides;
///
/// fn doc(overrides: std::collections::BTreeMap<String, Visibility>) -> Document {
///     let mut permissions = PermissionSet::default();
///     permissions.property_overrides = overrides;
///     Document {
///         id: uuid::Uuid::new_v4(),
///         scope: Scope::Compendium { pack: "core".into() },
///         doc_type: "note".into(),
///         schema_version: 1,
///         name: None,
///         source: None,
///         base: None,
///         owner: None,
///         permissions,
///         embedded: Default::default(),
///         parent_id: None,
///         engine: None,
///         system: serde_json::json!({}),
///         created_at: 0,
///         updated_at: 0,
///     }
/// }
///
/// let ok = [("/system/secret".to_string(), Visibility::GmOnly)].into_iter().collect();
/// assert!(validate_property_overrides(&doc(ok)).is_ok());
///
/// // `/owner` is structural, not a redactable content band.
/// let bad = [("/owner".to_string(), Visibility::GmOnly)].into_iter().collect();
/// assert!(validate_property_overrides(&doc(bad)).is_err());
/// ```
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

/// Placement rules that need no database: a `combat` is never parented and
/// never embedded; a `table` is likewise never parented and never embedded;
/// a `combatant`/`combat-history` is always parented (its
/// parent must be a `combat`, checked at the persistence chokepoint where
/// the parent can be loaded) and never embedded; an `asset_folder` is never
/// embedded (its parent-type rule lives at the persistence chokepoint,
/// `SqliteRepository::check_asset_folder_parent`; the ancestor cycle walk
/// for a Move is `SqliteRepository::check_move_acyclic`). Recurses into
/// every embedded descendant.
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::validation::validate_containment;
///
/// fn doc(doc_type: &str, parent_id: Option<uuid::Uuid>) -> Document {
///     Document {
///         id: uuid::Uuid::new_v4(),
///         scope: Scope::Compendium { pack: "core".into() },
///         doc_type: doc_type.into(),
///         schema_version: 1,
///         name: None,
///         source: None,
///         base: None,
///         owner: None,
///         permissions: PermissionSet::default(),
///         embedded: Default::default(),
///         parent_id,
///         engine: None,
///         system: serde_json::json!({}),
///         created_at: 0,
///         updated_at: 0,
///     }
/// }
///
/// assert!(validate_containment(&doc("combat", None)).is_ok());
/// assert!(validate_containment(&doc("combat", Some(uuid::Uuid::new_v4()))).is_err());
/// ```
pub fn validate_containment(doc: &Document) -> Result<(), DataError> {
    match doc.doc_type.as_str() {
        t if t == engine::COMBAT_DOC_TYPE && doc.parent_id.is_some() => {
            return Err(DataError::OpFailed(
                "a combat document cannot have a parent".into(),
            ));
        }
        t if t == engine::TABLE_DOC_TYPE && doc.parent_id.is_some() => {
            return Err(DataError::OpFailed(
                "a table document cannot have a parent".into(),
            ));
        }
        t if (t == engine::COMBATANT_DOC_TYPE || t == engine::COMBAT_HISTORY_DOC_TYPE)
            && doc.parent_id.is_none() =>
        {
            return Err(DataError::OpFailed(format!(
                "a '{t}' document requires a parent combat"
            )));
        }
        _ => {}
    }
    for children in doc.embedded.values() {
        for child in children {
            if child.doc_type == engine::COMBAT_DOC_TYPE
                || child.doc_type == engine::COMBATANT_DOC_TYPE
                || child.doc_type == engine::COMBAT_HISTORY_DOC_TYPE
                || child.doc_type == engine::ASSET_FOLDER_DOC_TYPE
                || child.doc_type == engine::TABLE_DOC_TYPE
                || child.doc_type == engine::NOTE_DOC_TYPE
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
