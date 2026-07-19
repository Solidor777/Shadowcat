# M13f — Server Declarative Schema Registry (tier-2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the server a structural (shape-only) enforcement floor over module-declared subtrees of the opaque `system` body, expressed as declarative data the server interprets but never executes — closing tier-2 of the phased-validation model.

**Architecture:** A GM-controlled, per-world registry of `SchemaDeclaration`s (module-keyed, `(doc_type, /system/… pointer) → Schema`), committed via a GM-only HTTP endpoint and broadcast in `Welcome` (informational parity). A new read-only recursive validator `validate_system_schema_tree` runs in `apply_intent` — Phase-1 for `Create` post-images and Phase-2 for `Update` post-images, immediately after `validate_engine_tree` — against the pre-transaction-loaded set. A violation rejects the whole intent via a new `DataError::SchemaViolation` that rides the existing rejected-intent path (no new wire frame). The declarable `Schema` is a recursive JSON type-tree (`type`/`properties`/`required`/`items`/`additionalProperties`/`nullable`) and nothing more, so it is incapable of expressing a value rule by construction.

**Tech Stack:** Rust (server), serde/serde_json, ts-rs (wire-type source of truth), axum (HTTP), sqlx/SQLite (world settings storage), Svelte 5 client with Zod mirror (`@shadowcat/core`).

## Model/Effort directives

Plan authored by sdd-plan-writer-opus (opus/high). Dispatcher: mainline session; the user will set the SDD-execution model/effort at the pause after this plan (M13e precedent: Sonnet 5/medium dispatcher). Implementers: shadowcat-coder (sonnet/medium) per task; escalate to shadowcat-coder-opus on BLOCKED. Reviewers: shadowcat-spec-reviewer + shadowcat-code-reviewer (opus/high) as the two-reviewer pair at task gates; final whole-branch review via the -opus twins.

## Buddy-check directives

Two tasks carry M13f's only new server enforcement surface (spec §8) and are pre-authorized for a **security-lens buddy-check** — two independent blind reviewers replacing the ordinary two-reviewer pair, each reviewing without seeing the other's findings:

- **Task 5 (apply_intent Phase-1/Phase-2 wiring)** — the enforcement chokepoint. Seam-scoping rationale: this is the single ingress gate where a write's post-image is judged; a placement error (wrong phase, wrong image, tx not dropped on failure) is a silent integrity hole invisible to unit tests of the validator alone.
- **Task 6 (set-time `validate_schema_declarations` + `validate_schema` + GM-only endpoints)** — the authority seam. Seam-scoping rationale: pointer-scoping (strict `/system` descendant), overlap/duplicate rejection, and `require_gm` gating are the load-bearing checks that stop a permissive/ambiguous schema or a non-GM writer from subverting the whole gate; overlap logic in particular is easy to get subtly wrong (prefix vs equality).

Tasks 1–4, 7–9 use the ordinary shadowcat-spec-reviewer + shadowcat-code-reviewer pair. This task adds no new egress, no new frame, no formula/notation surface.

## Global Constraints

- **Invariant 6 (structural-only).** Tier-2 validates SHAPE ONLY, never values. The schema vocabulary is incapable of expressing a value rule by construction. No semantic/mechanical validation of `system` content, ever.
- **Zero new wire frames, zero new operation types.** Rejection rides the existing rejected-intent path (like `BadEngine`/OCC `Conflict`). The `Welcome` schema-set is informational parity only.
- **Cross-platform, single binary, no third-party code on the server.** `std::path` only; the server stores & interprets declarative schema DATA but never loads/executes module code.
- **Authority = GM-controlled world state.** The document writer NEVER supplies the schema that judges it. Set endpoints are GM/admin-only (`require_gm`).
- **ts-rs is the source of truth for wire types.** Change the Rust type, regenerate, mirror in the client Zod schema; a drift guard enforces parity. Never hand-edit `src/types/generated`.
- **OCC pre-image discipline.** Do not disturb `apply_intent`'s pre-image comparison; every `FieldChange.old` stays the raw stored value.
- **Enforce-on-write, latest-wins, no retroactive/migration/per-doc-version routing.** (F6)
- **Build gate:** `cargo build --all-targets` (NOT just `--lib`) and `cargo test` from `src/server/` — `--lib` alone misses `src/bin/`/`tests/` targets; a missing struct-literal field there is a Critical build break invisible to `--lib` (known M13-0 lesson). Client: `pnpm -r typecheck` (full repo, not one package — a required field added to a shared type breaks cross-package fixtures) and `pnpm -r test`.

## File structure

| File | Responsibility | Task |
|---|---|---|
| `src/server/src/data/document.rs` | `SchemaType`, `AdditionalProperties`, `Schema`, `SchemaDeclaration` types (ts-rs-exported) | 1 |
| `src/types/index.ts` | Barrel re-export of the four new generated types | 1 |
| `src/server/src/data/validation.rs` | `validate_value_against_schema` matcher; `validate_system_schema_tree` recursive tree validator | 2, 3 |
| `src/server/src/data/mod.rs` | `DataError::SchemaViolation` variant | 3 |
| `src/server/src/http/error.rs` | `SchemaViolation → AppError::Unprocessable` mapping | 3 |
| `src/server/src/data/repository.rs` | `world_schema_declarations` trait read method | 4 |
| `src/server/src/data/sqlite.rs` | `set_world_schema_declarations` writer + trait read impl + `world_schemas_key`; `apply_intent` wiring | 4, 5 |
| `src/server/src/http/routes.rs` | `validate_schema`, `validate_schema_declarations`, GM-only get/set handlers | 6 |
| `src/server/src/http/mod.rs` | Route registration for `/api/worlds/{id}/schemas` | 6 |
| `src/server/src/ws/protocol.rs` | `Welcome.schema_declarations` field | 7 |
| `src/server/src/ws/conn.rs` | Load + include schema set in `Welcome` | 7 |
| `src/client/core/src/wire.ts` | Zod mirror `SchemaTypeSchema`/`SchemaSchema`/`SchemaDeclarationSchema` + welcome field | 8 |
| `src/client/core/src/wire.test.ts` | Drift-guard type equality for the new wire types | 8 |
| `docs/PLAN.md`, `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md` | M13f → done; tier-2 seam + invariant | 9 |

---

### Task 1: Schema grammar + declaration types (`document.rs`)

**Files:**
- Modify: `src/server/src/data/document.rs` (add types after `ContractDeclaration`, ~line 176; imports already present: `serde::{Deserialize, Serialize}`, `ts_rs::TS`, `std::collections::BTreeMap`)
- Modify: `src/types/index.ts` (barrel re-export)
- Test: inline `#[cfg(test)]` in `document.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `pub enum SchemaType { Object, Array, String, Number, Boolean, Null }` (serde `snake_case`)
  - `pub enum AdditionalProperties { Bool(bool), Schema(Box<Schema>) }` (untagged serialize; custom Deserialize)
  - `pub struct Schema { ty: Option<SchemaType>, properties: Option<BTreeMap<String, Schema>>, required: Option<Vec<String>>, additional_properties: Option<AdditionalProperties>, items: Option<Box<Schema>>, nullable: Option<bool> }`
  - `pub struct SchemaDeclaration { module_id: String, version: String, schema_format: u32, doc_type: String, subtree_pointer: String, schema: Schema }`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/server/src/data/document.rs`:

```rust
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
    assert!(matches!(s.additional_properties, Some(super::AdditionalProperties::Schema(_))));
}

#[test]
fn additional_properties_accepts_bool() {
    let s: super::Schema = serde_json::from_value(serde_json::json!({
        "type": "object", "additionalProperties": true
    }))
    .unwrap();
    assert!(matches!(s.additional_properties, Some(super::AdditionalProperties::Bool(true))));
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
    // silently dropped (mirrors the TokenVisual tagged-enum hole in validation.rs).
    assert!(serde_json::from_value::<super::Schema>(serde_json::json!({
        "type": "object",
        "additionalProperties": { "type": "string", "enum": ["a"] }
    }))
    .is_err());
}

#[test]
fn bad_schema_type_fails_to_deserialize() {
    assert!(serde_json::from_value::<super::Schema>(serde_json::json!({ "type": "integer" })).is_err());
}

#[test]
fn schema_declaration_round_trips_and_rejects_unknown_field() {
    let d: super::SchemaDeclaration = serde_json::from_value(serde_json::json!({
        "module_id": "nightfox", "version": "1.0.0", "schema_format": 1,
        "doc_type": "actor", "subtree_pointer": "/system/stats",
        "schema": { "type": "object" }
    }))
    .unwrap();
    assert_eq!(d.module_id, "nightfox");
    let s = serde_json::to_string(&d).unwrap();
    let back: super::SchemaDeclaration = serde_json::from_str(&s).unwrap();
    assert_eq!(d, back);
    // deny_unknown_fields on the declaration envelope.
    assert!(serde_json::from_value::<super::SchemaDeclaration>(serde_json::json!({
        "module_id": "n", "version": "1", "schema_format": 1, "doc_type": "actor",
        "subtree_pointer": "/system/x", "schema": {}, "bogus": 1
    }))
    .is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/server && cargo test -p shadowcat data::document::tests 2>&1 | tail -20`
Expected: FAIL — `cannot find type Schema / SchemaType / SchemaDeclaration in this scope`.

- [ ] **Step 3: Write the types**

Insert into `src/server/src/data/document.rs` immediately after the `ContractDeclaration` struct (after ~line 176), before `PermissionSet`:

```rust
/// A single JSON type tag for a schema node (M13f tier-2). Shape only — never a
/// value discriminator (invariant 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum SchemaType {
    Object,
    Array,
    String,
    Number,
    Boolean,
    Null,
}

/// `additionalProperties`: a bool (`false` = closed, `true` = any) or a subschema
/// every non-`properties` key must match. Serialized untagged (`boolean | Schema`);
/// the hand-written `Deserialize` routes a JSON object straight into `Schema` via
/// `MapAccessDeserializer` so the inner schema's `deny_unknown_fields` is enforced
/// (an untagged/internally-tagged derive would buffer through `Content` and drop
/// that check — the same serde limitation documented for `TokenVisual` in
/// `validation.rs`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum AdditionalProperties {
    Bool(bool),
    Schema(Box<Schema>),
}

impl<'de> Deserialize<'de> for AdditionalProperties {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ApVisitor;
        impl<'de> serde::de::Visitor<'de> for ApVisitor {
            type Value = AdditionalProperties;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a boolean or a schema object")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
                Ok(AdditionalProperties::Bool(v))
            }
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let schema =
                    Schema::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(AdditionalProperties::Schema(Box::new(schema)))
            }
        }
        deserializer.deserialize_any(ApVisitor)
    }
}

/// A structural (shape-only) type-tree node (M13f tier-2). By construction cannot
/// express a value rule (no enum/bounds/pattern/combinators) — invariant 6 holds
/// by construction. `deny_unknown_fields` makes a malformed schema fail to
/// deserialize at the set endpoint. An all-absent node (`{}`) matches any JSON.
/// Cross-field legality (e.g. `items` only on an array) is not enforced by serde;
/// `validate_schema` (routes.rs) enforces it at set-time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(deny_unknown_fields)]
pub struct Schema {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ty: Option<SchemaType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub properties: Option<BTreeMap<String, Schema>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub required: Option<Vec<String>>,
    #[serde(
        rename = "additionalProperties",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional, type = "boolean | Schema")]
    pub additional_properties: Option<AdditionalProperties>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub items: Option<Box<Schema>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub nullable: Option<bool>,
}

/// A module's per-`(doc_type, subtree)` structural schema (M13f tier-2). Pure
/// data — the server stores and interprets it as a shape check, never as code.
/// `subtree_pointer` is a strict `/system/…` descendant (enforced at set-time).
/// `schema_format` is the engine-owned vocabulary version; `version` is the
/// module's content version (provenance only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(deny_unknown_fields)]
pub struct SchemaDeclaration {
    pub module_id: String,
    pub version: String,
    pub schema_format: u32,
    pub doc_type: String,
    pub subtree_pointer: String,
    pub schema: Schema,
}
```

- [ ] **Step 4: Run tests to verify they pass and regenerate ts-rs bindings**

Run: `cd src/server && cargo test -p shadowcat data::document::tests 2>&1 | tail -20`
Expected: PASS. ts-rs writes `src/types/generated/SchemaType.ts`, `Schema.ts`, `SchemaDeclaration.ts` as a side effect of the export tests (`#[ts(export)]`).

Verify generation: `ls ../types/generated/Schema.ts ../types/generated/SchemaType.ts ../types/generated/SchemaDeclaration.ts`
Expected: all three exist.

- [ ] **Step 5: Add barrel re-exports**

In `src/types/index.ts`, after the `ContractDeclaration` export (line 29), add:

```ts
export type { SchemaType } from "./generated/SchemaType";
export type { Schema } from "./generated/Schema";
export type { SchemaDeclaration } from "./generated/SchemaDeclaration";
```

- [ ] **Step 6: Commit**

```bash
git add src/server/src/data/document.rs src/types/generated/Schema.ts src/types/generated/SchemaType.ts src/types/generated/SchemaDeclaration.ts src/types/index.ts
git commit -m "feat(m13f): Schema grammar + SchemaDeclaration wire types (ts-rs)"
```

---

### Task 2: Pure value matcher `validate_value_against_schema` (`validation.rs`)

**Files:**
- Modify: `src/server/src/data/validation.rs` (add matcher + helpers; extend `use` for the new types)
- Test: inline `#[cfg(test)]` in `validation.rs`

**Interfaces:**
- Consumes: `Schema`, `SchemaType`, `AdditionalProperties` (Task 1).
- Produces:
  - `pub struct SchemaMismatch { pub pointer: String, pub reason: String }`
  - `pub fn validate_value_against_schema(value: &serde_json::Value, schema: &Schema) -> Result<(), SchemaMismatch>` — pointer is relative to `value`'s root (empty string at the root); `reason` is a structural phrase (`expected number, got string`, `unknown key '…' not permitted by schema`, `missing required key '…'`). Matches purely on JSON type — never inspects a value's magnitude/content (invariant 6). Number matching is `Value::is_number` only, so the PosInt/Float variant split is irrelevant here.

- [ ] **Step 1: Write the failing tests**

Add a new test module block at the end of `src/server/src/data/validation.rs`'s `tests` module (inside `mod tests`, after the existing tests):

```rust
// --- validate_value_against_schema: accept/reject matrix (M13f tier-2) ---

use crate::data::document::{AdditionalProperties, Schema, SchemaType};

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
    assert!(validate_value_against_schema(&serde_json::json!({ "a": [1, "b", null] }), &any).is_ok());
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
    let err = validate_value_against_schema(&serde_json::json!({ "a": 1, "b": 2 }), &s).unwrap_err();
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
    let err = validate_value_against_schema(
        &serde_json::json!({ "str": { "kind": 5 } }),
        &s,
    )
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
    let s: Schema = obj_schema(serde_json::json!({ "type": "array", "items": { "type": "number" } }));
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/server && cargo test -p shadowcat data::validation::tests::scalar_type_match_and_mismatch 2>&1 | tail -20`
Expected: FAIL — `cannot find function validate_value_against_schema`.

- [ ] **Step 3: Write the matcher**

At the top of `src/server/src/data/validation.rs`, extend the `use` block:

```rust
use crate::data::document::{AdditionalProperties, Document, Schema, SchemaType};
```

(Replace the existing `use crate::data::document::Document;` line — keep `use crate::data::engine;` and `use crate::data::DataError;` as-is.)

Add, after `validate_engine_tree` (after ~line 79) and before `validate_field_path`:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/server && cargo test -p shadowcat data::validation::tests 2>&1 | tail -20`
Expected: PASS (all matrix tests green).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/validation.rs
git commit -m "feat(m13f): pure structural value matcher (accept/reject matrix)"
```

---

### Task 3: Recursive tree validator + `DataError::SchemaViolation` (`validation.rs`, `mod.rs`, `http/error.rs`)

**Files:**
- Modify: `src/server/src/data/mod.rs` (add `SchemaViolation` variant)
- Modify: `src/server/src/http/error.rs` (map new variant)
- Modify: `src/server/src/data/validation.rs` (`validate_system_schema_tree`)
- Test: inline `#[cfg(test)]` in `validation.rs`

**Interfaces:**
- Consumes: `validate_value_against_schema` / `SchemaMismatch` (Task 2); `SchemaDeclaration` (Task 1); `Document` (existing).
- Produces:
  - `DataError::SchemaViolation { pointer: String, reason: String }`
  - `pub fn validate_system_schema_tree(doc: &Document, schemas: &[SchemaDeclaration]) -> Result<(), DataError>` — read-only (no normalization); for each declaration whose `doc_type` matches `doc.doc_type`, resolves `subtree_pointer` inside `doc.system` (an absent subtree is NOT a violation); recurses `doc.embedded` looking each child up by its OWN `doc_type`. On mismatch returns `SchemaViolation` with `pointer` = `subtree_pointer` concatenated with the relative mismatch pointer.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module of `src/server/src/data/validation.rs`:

```rust
// --- validate_system_schema_tree: subtree scoping, embedded, absent-ok ---

use crate::data::document::SchemaDeclaration;

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
    let schemas = vec![decl("actor", "/system/stats", serde_json::json!({ "type": "object" }))];
    assert!(validate_system_schema_tree(&doc, &schemas).is_ok());
}

#[test]
fn tree_validator_unregistered_doc_type_passes() {
    let doc = doc_with_system(serde_json::json!({ "anything": true }));
    let schemas = vec![decl("item", "/system/x", serde_json::json!({ "type": "number" }))];
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
        decl("actor", "/system/stats", serde_json::json!({ "type": "object",
            "additionalProperties": { "type": "object",
                "properties": { "kind": { "type": "string" } } } })),
        decl("actor", "/system/mechanics", serde_json::json!({ "type": "object",
            "required": ["version"], "properties": { "version": { "type": "number" } } })),
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/server && cargo test -p shadowcat data::validation::tests::tree_validator_absent_subtree_is_ok 2>&1 | tail -20`
Expected: FAIL — `no variant SchemaViolation` / `cannot find function validate_system_schema_tree`.

- [ ] **Step 3a: Add the `DataError` variant**

In `src/server/src/data/mod.rs`, add to the `DataError` enum after `BadEngine(String)` (line 37):

```rust
    #[error("schema violation at {pointer}: {reason}")]
    SchemaViolation { pointer: String, reason: String },
```

- [ ] **Step 3b: Map it in `http/error.rs`**

In `src/server/src/http/error.rs`, add an arm to the `From<crate::data::DataError>` match after the `BadEngine(m)` arm (line 32):

```rust
            SchemaViolation { pointer, reason } => {
                AppError::Unprocessable(format!("schema violation at {pointer}: {reason}"))
            }
```

- [ ] **Step 3c: Write the tree validator**

In `src/server/src/data/validation.rs`, extend the top `use` to include `SchemaDeclaration`:

```rust
use crate::data::document::{AdditionalProperties, Document, Schema, SchemaDeclaration, SchemaType};
```

Add after `validate_value_against_schema`'s helpers (after `check_value`):

```rust
/// Validate the POST-IMAGE `system` band against the world's registered
/// structural schemas (M13f tier-2), recursing embedded descendants — each
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/server && cargo test -p shadowcat data::validation::tests 2>&1 | tail -20`
Expected: PASS.
Then: `cd src/server && cargo build --all-targets 2>&1 | tail -20`
Expected: builds (confirms the exhaustive `From<DataError>` match now covers `SchemaViolation`).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/validation.rs src/server/src/data/mod.rs src/server/src/http/error.rs
git commit -m "feat(m13f): recursive system-schema tree validator + SchemaViolation error"
```

---

### Task 4: Repository read/write pair + storage (`repository.rs`, `sqlite.rs`)

**Files:**
- Modify: `src/server/src/data/repository.rs` (trait read method + import)
- Modify: `src/server/src/data/sqlite.rs` (inherent writer, trait read impl, settings key, import)
- Test: inline `#[cfg(test)]` in `sqlite.rs`

**Interfaces:**
- Consumes: `SchemaDeclaration` (Task 1).
- Produces:
  - trait method `async fn world_schema_declarations(&self, world: Uuid) -> Result<Vec<SchemaDeclaration>, DataError>;` (empty vec when unset)
  - inherent `pub async fn set_world_schema_declarations(&self, world: Uuid, decls: &[SchemaDeclaration]) -> Result<(), DataError>`
  - `fn world_schemas_key(world: Uuid) -> String` → `"world_schemas:{world}"`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module of `src/server/src/data/sqlite.rs` (near `contract_declarations_round_trip_and_default_empty`, ~line 2295):

```rust
#[tokio::test]
async fn schema_declarations_round_trip_and_default_empty() {
    use crate::data::document::{Schema, SchemaDeclaration, SchemaType};
    let repo = repo().await;
    let world = repo
        .create_world("W", Uuid::from_u128(1))
        .await
        .unwrap();
    // Default empty.
    assert!(repo
        .world_schema_declarations(world.id)
        .await
        .unwrap()
        .is_empty());

    let decls = vec![SchemaDeclaration {
        module_id: "nightfox".into(),
        version: "1.0.0".into(),
        schema_format: 1,
        doc_type: "actor".into(),
        subtree_pointer: "/system/stats".into(),
        schema: Schema {
            ty: Some(SchemaType::Object),
            ..Default::default()
        },
    }];
    repo.set_world_schema_declarations(world.id, &decls)
        .await
        .unwrap();
    let got = repo.world_schema_declarations(world.id).await.unwrap();
    assert_eq!(got, decls);
}
```

(Confirm the exact `create_world` signature used by the neighboring contract test and mirror it; if that test uses `repo.create_world("W").await` or a helper, use the identical call.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/server && cargo test -p shadowcat schema_declarations_round_trip 2>&1 | tail -20`
Expected: FAIL — `no method world_schema_declarations` / `set_world_schema_declarations`.

- [ ] **Step 3a: Add the trait read method**

In `src/server/src/data/repository.rs`, extend the import (line 5-7):

```rust
use crate::data::document::{
    CapabilityRequirement, ContractDeclaration, Document, SchemaDeclaration, World,
    WorldCapDefaults, WorldRole,
};
```

Add to the `Repository` trait after `world_contract_declarations` (~line 94):

```rust
    /// A world's declarative structural schema declarations (GM-committed on
    /// module enable). Empty when unset.
    async fn world_schema_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<SchemaDeclaration>, DataError>;
```

- [ ] **Step 3b: Add the settings key, writer, and trait impl in `sqlite.rs`**

Ensure `SchemaDeclaration` is imported in `sqlite.rs` (add it to the existing `use crate::data::document::{...}` list).

Add the inherent writer after `set_world_contract_declarations` (~line 594):

```rust
    /// Replace a world's structural schema declarations (stored as JSON in
    /// settings, beside cap requirements / contract declarations).
    pub async fn set_world_schema_declarations(
        &self,
        world: Uuid,
        decls: &[SchemaDeclaration],
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(decls)?;
        self.set_setting(&world_schemas_key(world), &json).await
    }
```

Add the trait read impl after `world_contract_declarations` (~line 1608):

```rust
    async fn world_schema_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<SchemaDeclaration>, DataError> {
        match self.get_setting(&world_schemas_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
    }
```

Add the key helper after `world_contracts_key` (~line 1762):

```rust
/// Settings key holding a world's structural schema declarations (JSON).
fn world_schemas_key(world: Uuid) -> String {
    format!("world_schemas:{world}")
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src/server && cargo test -p shadowcat schema_declarations_round_trip 2>&1 | tail -20`
Expected: PASS.
Then confirm no other `Repository` impl broke: `cd src/server && cargo build --all-targets 2>&1 | tail -20`
Expected: builds (if a test-double `Repository` impl exists it must implement the new method; add a trivial `Ok(Vec::new())` impl there if the build flags one).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/repository.rs src/server/src/data/sqlite.rs
git commit -m "feat(m13f): world_schema_declarations repository read/write pair"
```

---

### Task 5: Wire the enforcement chokepoint into `apply_intent` (`sqlite.rs`) — BUDDY-CHECK (security)

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (`apply_intent`: pre-tx load + Create Phase-1 call + Update Phase-2 call)
- Test: inline `#[cfg(test)]` in `sqlite.rs`

**Interfaces:**
- Consumes: `validate_system_schema_tree` (Task 3); `world_schema_declarations` (Task 4).
- Produces: enforcement of the registered schema set on every `Create`/`Update` post-image; a violating write yields `DataError::SchemaViolation` and the transaction is dropped, so the per-world seq is not consumed.

**Placement note (resolved decision):** `validate_engine_tree` is itself split across phases in `apply_intent` — Create validates its post-image in Phase-1 (~line 1057); Update's merged post-image only exists in Phase-2 (~line 1396, after the field merge). The spec's "immediately after `validate_engine_tree`" therefore means **Create → Phase-1**, **Update → Phase-2**. Both preserve the "seq not consumed" invariant: a Phase-1 return drops the tx before the seq is allocated; a Phase-2 return drops the tx before `tx.commit()`, reverting the `worlds.seq` increment.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module of `src/server/src/data/sqlite.rs`. Model these on the existing `apply_intent` tests (reuse their world/user/ctx setup helpers — mirror the nearest `apply_intent_*` test's construction of `PermissionContext`, `Operation::Create`, and the GM role, and set the world's schema set with `set_world_schema_declarations` before the intent):

```rust
#[tokio::test]
async fn apply_intent_create_violating_system_schema_is_rejected_and_seq_untouched() {
    use crate::data::document::{Schema, SchemaDeclaration, SchemaType};
    // ... build repo + world + GM ctx exactly as the neighboring apply_intent tests do ...
    // Register: actor /system/mechanics requires object with numeric `version`.
    let decls = vec![SchemaDeclaration {
        module_id: "nightfox".into(), version: "1".into(), schema_format: 1,
        doc_type: "actor".into(), subtree_pointer: "/system/mechanics".into(),
        schema: serde_json::from_value(serde_json::json!({
            "type": "object", "required": ["version"],
            "properties": { "version": { "type": "number" } }
        })).unwrap(),
    }];
    repo.set_world_schema_declarations(world.id, &decls).await.unwrap();
    let seq_before = repo.get_world(world.id).await.unwrap().unwrap().seq;

    // A Create whose /system/mechanics.version is a string violates the schema.
    let mut doc = /* world_scoped_doc(world.id, Uuid::new_v4(), "actor") with a valid engine */;
    doc.system = serde_json::json!({ "mechanics": { "version": "oops" } });
    let err = repo
        .apply_intent(&gm_ctx, world.id, vec![Operation::Create { doc }], 1, WriteOrigin::Client)
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::SchemaViolation { .. }));
    // Rejected intent consumes no seq (transaction dropped).
    let seq_after = repo.get_world(world.id).await.unwrap().unwrap().seq;
    assert_eq!(seq_before, seq_after);
}

#[tokio::test]
async fn apply_intent_create_conforming_system_schema_succeeds() {
    use crate::data::document::{SchemaDeclaration};
    // ... same setup ...
    let decls = vec![SchemaDeclaration {
        module_id: "nightfox".into(), version: "1".into(), schema_format: 1,
        doc_type: "actor".into(), subtree_pointer: "/system/mechanics".into(),
        schema: serde_json::from_value(serde_json::json!({
            "type": "object", "required": ["version"],
            "properties": { "version": { "type": "number" } }
        })).unwrap(),
    }];
    repo.set_world_schema_declarations(world.id, &decls).await.unwrap();
    let mut doc = /* world_scoped_doc(...,"actor") with valid engine */;
    doc.system = serde_json::json!({ "mechanics": { "version": 2 } });
    assert!(repo
        .apply_intent(&gm_ctx, world.id, vec![Operation::Create { doc }], 1, WriteOrigin::Client)
        .await
        .is_ok());
}

#[tokio::test]
async fn apply_intent_update_violating_system_schema_is_rejected_and_seq_untouched() {
    use crate::data::command::FieldChange;
    use crate::data::document::SchemaDeclaration;
    // ... setup: create a conforming actor with /system/mechanics = { version: 1 } ...
    let decls = vec![SchemaDeclaration {
        module_id: "nightfox".into(), version: "1".into(), schema_format: 1,
        doc_type: "actor".into(), subtree_pointer: "/system/mechanics".into(),
        schema: serde_json::from_value(serde_json::json!({
            "type": "object", "required": ["version"],
            "properties": { "version": { "type": "number" } }
        })).unwrap(),
    }];
    repo.set_world_schema_declarations(world.id, &decls).await.unwrap();
    // Create a conforming doc first (records the seq), then Update version to a string.
    // ... perform conforming Create via apply_intent ...
    let seq_before = repo.get_world(world.id).await.unwrap().unwrap().seq;
    let update = Operation::Update {
        doc_id: doc_id,
        changes: vec![FieldChange {
            path: "/system/mechanics/version".into(),
            old: serde_json::json!(1),
            new: serde_json::json!("oops"),
        }],
    };
    let err = repo
        .apply_intent(&gm_ctx, world.id, vec![update], 2, WriteOrigin::Client)
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::SchemaViolation { .. }));
    let seq_after = repo.get_world(world.id).await.unwrap().unwrap().seq;
    assert_eq!(seq_before, seq_after);
}
```

Fill the `/* ... */` scaffolding by copying the exact setup from the nearest existing `apply_intent_*` test (e.g. `apply_intent_create_then_conflicting_update`, `apply_intent_world_default_grants_apply`) so the ctx/world/doc builders match the codebase precisely.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/server && cargo test -p shadowcat apply_intent_create_violating_system_schema 2>&1 | tail -20`
Expected: FAIL — the create currently succeeds (no schema gate) so `unwrap_err` panics.

- [ ] **Step 3a: Load the schema set before the transaction**

In `apply_intent`, after the `world_reqs` load and before `let mut tx = self.pool.begin().await?;` (~line 1042):

```rust
        // Loaded before the transaction (like world_cap_requirements): the
        // single-writer pool would deadlock on a mid-tx settings query. This is
        // the GM-controlled tier-2 registry; the writer never supplies its own
        // judging schema.
        let world_schemas = self.world_schema_declarations(world_id).await?;
```

- [ ] **Step 3b: Enforce on the Create post-image (Phase-1)**

In the `Operation::Create { doc }` arm, immediately after `validation::validate_engine_tree(doc)?;` (~line 1057):

```rust
                    validation::validate_system_schema_tree(doc, &world_schemas)?;
```

- [ ] **Step 3c: Enforce on the Update post-image (Phase-2)**

In the `Operation::Update { doc_id, changes }` arm of the Phase-2 loop, immediately after `validation::validate_engine_tree(&mut doc)?;` (~line 1396) and before `doc.updated_at = ts;`:

```rust
                    validation::validate_system_schema_tree(&doc, &world_schemas)?;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/server && cargo test -p shadowcat apply_intent 2>&1 | tail -25`
Expected: the three new tests PASS; all pre-existing `apply_intent_*` tests still PASS (docs with no registered schema for their doc_type write freely).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/sqlite.rs
git commit -m "feat(m13f): enforce system-schema tree in apply_intent (Create P1, Update P2)"
```

- [ ] **Step 6: BUDDY-CHECK (security-lens, two blind reviewers).** Verify: (a) the schema set is loaded pre-tx (no mid-tx deadlock); (b) Create is gated in Phase-1 and Update in Phase-2 with the merged post-image, never the pre-image; (c) both failure paths drop the transaction so the per-world seq is not consumed; (d) the validator is read-only (`&doc`) and does not disturb OCC pre-images or engine normalization; (e) an unregistered doc_type/subtree writes freely. Note the reject_reason mapping (`_ => Invalid` in `ws/conn.rs`) already covers `SchemaViolation` with no new frame.

---

### Task 6: Set-time validation + GM-only endpoints (`routes.rs`, `http/mod.rs`) — BUDDY-CHECK (security)

**Files:**
- Modify: `src/server/src/http/routes.rs` (constants, `validate_schema`, `validate_schema_declarations`, `get`/`set` handlers, import)
- Modify: `src/server/src/http/mod.rs` (route registration)
- Test: inline `#[cfg(test)]` in `http/mod.rs` (mirror `contract_declarations_gm_crud_and_validation`) or `routes.rs`

**Interfaces:**
- Consumes: `SchemaDeclaration`, `Schema`, `SchemaType`, `AdditionalProperties` (Task 1); `world_schema_declarations`/`set_world_schema_declarations` (Task 4); `require_gm` (existing).
- Produces:
  - `const MAX_SCHEMA_DECLARATIONS: usize = 256;`
  - `const MAX_SCHEMA_NODES: usize = 512;`
  - `const MAX_SCHEMA_DEPTH: usize = 16;`
  - `const SCHEMA_FORMAT_V1: u32 = 1;`
  - `fn validate_schema(s: &Schema, depth: usize, budget: &mut usize) -> Result<(), AppError>`
  - `fn validate_schema_declarations(decls: &[SchemaDeclaration]) -> Result<(), AppError>`
  - `pub async fn get_world_schema_declarations(...) -> Result<Json<Vec<SchemaDeclaration>>, AppError>`
  - `pub async fn set_world_schema_declarations(...) -> Result<StatusCode, AppError>`
  - route `GET/PUT /api/worlds/{id}/schemas`

- [ ] **Step 1: Write the failing tests**

Add to the test module of `src/server/src/http/mod.rs` (mirror `contract_declarations_gm_crud_and_validation` / `world_capability_requirements_gm_only_crud` for the harness, GM vs player clients, and the URL path):

```rust
#[tokio::test]
async fn schema_declarations_gm_crud_and_validation() {
    // ... build app + GM client `gm` + player client `pl` + a world_id ...
    let base = format!("/api/worlds/{world_id}/schemas");

    // Non-GM cannot read or write.
    assert_eq!(pl.get(&base).await.status_code(), 403);
    assert_eq!(
        pl.put(&base).json(&serde_json::json!([])).await.status_code(),
        403
    );

    // GM: empty set is valid; default read is empty.
    assert_eq!(gm.put(&base).json(&serde_json::json!([])).await.status_code(), 204);
    let got: Vec<serde_json::Value> = gm.get(&base).await.json();
    assert!(got.is_empty());

    // Valid declaration accepted.
    let ok = serde_json::json!([{
        "module_id": "nightfox", "version": "1.0.0", "schema_format": 1,
        "doc_type": "actor", "subtree_pointer": "/system/stats",
        "schema": { "type": "object", "additionalProperties": { "type": "object",
            "properties": { "kind": { "type": "string" } } } }
    }]);
    assert_eq!(gm.put(&base).json(&ok).await.status_code(), 204);

    // Pointer not a strict /system descendant → rejected.
    for bad_ptr in ["/engine/vision", "/permissions", "/name", "", "/system"] {
        let body = serde_json::json!([{
            "module_id": "m", "version": "1", "schema_format": 1, "doc_type": "actor",
            "subtree_pointer": bad_ptr, "schema": { "type": "object" }
        }]);
        assert_eq!(
            gm.put(&base).json(&body).await.status_code(), 422,
            "pointer {bad_ptr} must be rejected"
        );
    }

    // Overlapping pointers on one doc_type → rejected.
    let overlap = serde_json::json!([
        { "module_id": "a", "version": "1", "schema_format": 1, "doc_type": "actor",
          "subtree_pointer": "/system/stats", "schema": { "type": "object" } },
        { "module_id": "b", "version": "1", "schema_format": 1, "doc_type": "actor",
          "subtree_pointer": "/system/stats/str", "schema": { "type": "object" } }
    ]);
    assert_eq!(gm.put(&base).json(&overlap).await.status_code(), 422);

    // Duplicate module_id → rejected.
    let dup_mod = serde_json::json!([
        { "module_id": "a", "version": "1", "schema_format": 1, "doc_type": "actor",
          "subtree_pointer": "/system/x", "schema": {} },
        { "module_id": "a", "version": "2", "schema_format": 1, "doc_type": "actor",
          "subtree_pointer": "/system/y", "schema": {} }
    ]);
    assert_eq!(gm.put(&base).json(&dup_mod).await.status_code(), 422);

    // Unknown schema_format → rejected.
    let bad_fmt = serde_json::json!([{
        "module_id": "m", "version": "1", "schema_format": 999, "doc_type": "actor",
        "subtree_pointer": "/system/x", "schema": {}
    }]);
    assert_eq!(gm.put(&base).json(&bad_fmt).await.status_code(), 422);

    // Malformed schema (unknown key) → deserialize fails → 4xx (not 204).
    let malformed = serde_json::json!([{
        "module_id": "m", "version": "1", "schema_format": 1, "doc_type": "actor",
        "subtree_pointer": "/system/x", "schema": { "type": "string", "enum": ["a"] }
    }]);
    assert_ne!(gm.put(&base).json(&malformed).await.status_code(), 204);

    // Cross-field-illegal schema (items on an object) → rejected by validate_schema.
    let cross = serde_json::json!([{
        "module_id": "m", "version": "1", "schema_format": 1, "doc_type": "actor",
        "subtree_pointer": "/system/x",
        "schema": { "type": "object", "items": { "type": "number" } }
    }]);
    assert_eq!(gm.put(&base).json(&cross).await.status_code(), 422);
}
```

Match the exact test-harness API (client builder, `.json()`, `.await.status_code()`) used by `contract_declarations_gm_crud_and_validation` in the same file.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/server && cargo test -p shadowcat schema_declarations_gm_crud 2>&1 | tail -20`
Expected: FAIL — route not registered / handler missing.

- [ ] **Step 3a: Constants + `validate_schema` + `validate_schema_declarations`**

In `src/server/src/http/routes.rs`, extend the `document` import (line 17-20):

```rust
use crate::data::document::{
    AdditionalProperties, CapabilityRequirement, Cardinality, ContractDeclaration, Document, Schema,
    SchemaDeclaration, SchemaType, Scope, World, WorldCapDefaults, WorldRole,
};
```

Add after `validate_contract_declarations` (~line 744):

```rust
/// Upper bound on schema declarations stored per world (parsed on every write,
/// broadcast in `Welcome`). Sized like `MAX_CONTRACT_DECLARATIONS`.
const MAX_SCHEMA_DECLARATIONS: usize = 256;
/// Upper bound on nodes in a single schema type-tree (backstops a pathological
/// declaration; the size cap already bounds the DATA, this bounds the SCHEMA).
const MAX_SCHEMA_NODES: usize = 512;
/// Upper bound on schema type-tree nesting depth.
const MAX_SCHEMA_DEPTH: usize = 16;
/// The single schema-format vocabulary version this server understands (F7). A
/// future vocabulary bump increments this and the set endpoint rejects formats
/// it does not know, so a format bump can never be silently half-enforced.
const SCHEMA_FORMAT_V1: u32 = 1;

/// Structurally validate one schema type-tree node (M13f tier-2), fail-closed:
/// bounded depth/node-count and cross-field legality (a node's non-`type` keys
/// must match its `type`). serde's `deny_unknown_fields` already rejected unknown
/// keys and bad `type` values at deserialize; this rejects a well-typed-but-
/// nonsensical node (e.g. `items` on an object, `properties` on an array).
fn validate_schema(s: &Schema, depth: usize, budget: &mut usize) -> Result<(), AppError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(AppError::Unprocessable(format!(
            "schema nesting exceeds max depth {MAX_SCHEMA_DEPTH}"
        )));
    }
    if *budget == 0 {
        return Err(AppError::Unprocessable(format!(
            "schema exceeds max node count {MAX_SCHEMA_NODES}"
        )));
    }
    *budget -= 1;

    let has_object_keys =
        s.properties.is_some() || s.required.is_some() || s.additional_properties.is_some();
    let has_array_keys = s.items.is_some();

    match s.ty {
        None => {
            // `{}` (any): must carry no other constraint keys.
            if has_object_keys || has_array_keys || s.nullable.is_some() {
                return Err(AppError::Unprocessable(
                    "a typeless schema node ({}) must have no other keys".into(),
                ));
            }
        }
        Some(SchemaType::Object) => {
            if has_array_keys {
                return Err(AppError::Unprocessable(
                    "'items' is only valid on an array schema".into(),
                ));
            }
            if let Some(props) = &s.properties {
                for sub in props.values() {
                    validate_schema(sub, depth + 1, budget)?;
                }
            }
            if let Some(AdditionalProperties::Schema(sub)) = &s.additional_properties {
                validate_schema(sub, depth + 1, budget)?;
            }
        }
        Some(SchemaType::Array) => {
            if has_object_keys {
                return Err(AppError::Unprocessable(
                    "'properties'/'required'/'additionalProperties' are only valid on an object schema"
                        .into(),
                ));
            }
            if let Some(items) = &s.items {
                validate_schema(items, depth + 1, budget)?;
            }
        }
        Some(SchemaType::String | SchemaType::Number | SchemaType::Boolean | SchemaType::Null) => {
            if has_object_keys || has_array_keys {
                return Err(AppError::Unprocessable(
                    "a scalar schema node accepts only 'type' and 'nullable'".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Validate a world's schema declaration set (M13f tier-2 set-time gate),
/// fail-closed — the server is the consistency authority. Mirrors
/// `validate_contract_declarations`: bounded count, non-empty module_id/version,
/// unique module_id, understood schema_format, strict `/system/…` pointers, no
/// duplicate/overlapping `(doc_type, pointer)`, and each schema a well-formed
/// type-tree.
fn validate_schema_declarations(decls: &[SchemaDeclaration]) -> Result<(), AppError> {
    use std::collections::HashSet;
    if decls.len() > MAX_SCHEMA_DECLARATIONS {
        return Err(AppError::Unprocessable(format!(
            "too many schema declarations (max {MAX_SCHEMA_DECLARATIONS})"
        )));
    }
    let mut seen_modules: HashSet<&str> = HashSet::new();
    for d in decls {
        if d.module_id.is_empty() || d.version.is_empty() {
            return Err(AppError::Unprocessable(
                "schema declaration module_id and version must be non-empty".into(),
            ));
        }
        if !seen_modules.insert(d.module_id.as_str()) {
            return Err(AppError::Unprocessable(format!(
                "duplicate module_id '{}'",
                d.module_id
            )));
        }
        if d.schema_format != SCHEMA_FORMAT_V1 {
            return Err(AppError::Unprocessable(format!(
                "unsupported schema_format {} (server understands {SCHEMA_FORMAT_V1})",
                d.schema_format
            )));
        }
        // Pointer must be a strict `/system/…` descendant: guards content only,
        // never the whole `system` body (bare `/system` re-introduces the
        // deny_unknown_fields-on-system problem the band split avoids), and never
        // `/engine`, `/permissions`, `/name`, or the envelope. Mirrors the
        // `set_world_cap_requirements` writable-namespace check, narrowed.
        let p = &d.subtree_pointer;
        if !p.starts_with("/system/") || p.ends_with('/') || p.contains("//") {
            return Err(AppError::Unprocessable(format!(
                "subtree_pointer '{p}' must be a strict /system/… descendant"
            )));
        }
        let mut budget = MAX_SCHEMA_NODES;
        validate_schema(&d.schema, 0, &mut budget)?;
    }
    // No two entries for one doc_type whose pointers are equal or where one is a
    // prefix of the other (ambiguous authority: which schema governs the nested
    // value?). Prefix test: `a` covers `b` iff `b == a` or `b` starts with
    // `a` + "/".
    for (i, a) in decls.iter().enumerate() {
        for b in &decls[i + 1..] {
            if a.doc_type != b.doc_type {
                continue;
            }
            let (x, y) = (&a.subtree_pointer, &b.subtree_pointer);
            let overlaps = x == y
                || y.starts_with(&format!("{x}/"))
                || x.starts_with(&format!("{y}/"));
            if overlaps {
                return Err(AppError::Unprocessable(format!(
                    "overlapping schema pointers '{x}' and '{y}' for doc_type '{}'",
                    a.doc_type
                )));
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 3b: Handlers**

Add after `set_world_contract_declarations` (~line 770):

```rust
/// A world's structural schema declarations. GM/admin only.
pub async fn get_world_schema_declarations(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<Vec<SchemaDeclaration>>, AppError> {
    require_gm(&state, &user, world).await?;
    Ok(Json(state.repo.world_schema_declarations(world).await?))
}

/// Replace a world's structural schema declarations. GM/admin only; validated.
pub async fn set_world_schema_declarations(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(decls): Json<Vec<SchemaDeclaration>>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    validate_schema_declarations(&decls)?;
    state
        .repo
        .set_world_schema_declarations(world, &decls)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 3c: Register the route**

In `src/server/src/http/mod.rs`, add after the `/api/worlds/{id}/contracts` route (~line 100):

```rust
        .route(
            "/api/worlds/{id}/schemas",
            get(routes::get_world_schema_declarations)
                .put(routes::set_world_schema_declarations),
        )
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/server && cargo test -p shadowcat schema_declarations_gm_crud 2>&1 | tail -25`
Expected: PASS.
Then: `cd src/server && cargo build --all-targets 2>&1 | tail -20`
Expected: builds.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/http/routes.rs src/server/src/http/mod.rs
git commit -m "feat(m13f): GM-only schema-declarations endpoint + set-time validation"
```

- [ ] **Step 6: BUDDY-CHECK (security-lens, two blind reviewers).** Verify: (a) both handlers call `require_gm` before any read/write (the writer never judges itself); (b) pointer scoping rejects `""`, `/system`, `/engine…`, `/permissions`, `/name`, trailing-slash, and empty-token pointers, and accepts strict `/system/…` descendants; (c) overlap/duplicate detection covers equality AND prefix in BOTH directions and is scoped per doc_type; (d) unknown `schema_format` and cross-field-illegal schemas are rejected; (e) bounds (count/depth/node-count) are enforced; (f) a malformed schema fails at deserialize (400/422) rather than storing.

---

### Task 7: Include the schema set in `Welcome` (`protocol.rs`, `conn.rs`)

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (`Welcome` variant + its test literal)
- Modify: `src/server/src/ws/conn.rs` (load + assemble)
- Modify: any other `ServerMsg::Welcome { … }` construction site (grep — the M13-0 struct-literal-break lesson)
- Test: extend the existing `welcome_carries_caps_role_and_requirements` test in `protocol.rs`

**Interfaces:**
- Consumes: `SchemaDeclaration` (Task 1); `world_schema_declarations` (Task 4).
- Produces: `Welcome.schema_declarations: Vec<crate::data::document::SchemaDeclaration>` (informational parity; NOT a client enforcement gate — tier-1 already validates client-side).

- [ ] **Step 1: Write the failing test**

Extend `welcome_carries_caps_role_and_requirements` in `src/server/src/ws/protocol.rs`: add `schema_declarations: Vec::new(),` to the struct literal and assert the field serializes:

```rust
        assert!(json.get("schema_declarations").is_some());
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/server && cargo test -p shadowcat welcome_carries_caps_role 2>&1 | tail -20`
Expected: FAIL — missing field `schema_declarations` in the `Welcome` initializer / assertion fails.

- [ ] **Step 3a: Add the field to the protocol type**

In `src/server/src/ws/protocol.rs`, add to the `Welcome { … }` variant after `contract_declarations` (~line 199):

```rust
        /// The world's structural schema declarations (tier-2), so the client
        /// can mirror expectations. Informational/parity only — tier-1 Zod
        /// validates client-side; this is NOT a client enforcement gate.
        schema_declarations: Vec<crate::data::document::SchemaDeclaration>,
```

- [ ] **Step 3b: Assemble it in `conn.rs`**

In `src/server/src/ws/conn.rs`, after the `world_contracts` load (~line 934) add:

```rust
    // Informational/parity for the client (tier-2 is server-enforced; tier-1
    // validates client-side). Fail open to empty for the advisory copy.
    let world_schemas = match repo.world_schema_declarations(world_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(world = %world_id, error = %e, "schema declarations unreadable; sending empty");
            Vec::new()
        }
    };
```

Then add to the `ServerMsg::Welcome { … }` literal after `contract_declarations: world_contracts,` (~line 948):

```rust
            schema_declarations: world_schemas,
```

- [ ] **Step 3c: Patch any other `Welcome` construction sites**

Run: `grep -rn "ServerMsg::Welcome {" src/server/src` and add `schema_declarations: Vec::new(),` (or the appropriate value) to every remaining literal the compiler flags.

- [ ] **Step 4: Run tests to verify they pass and regenerate ServerMsg bindings**

Run: `cd src/server && cargo test -p shadowcat welcome 2>&1 | tail -20`
Expected: PASS. ts-rs regenerates `src/types/generated/ServerMsg.ts` with the new welcome field.
Then: `cd src/server && cargo build --all-targets 2>&1 | tail -20`
Expected: builds (all Welcome literals updated).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/protocol.rs src/server/src/ws/conn.rs src/types/generated/ServerMsg.ts
git commit -m "feat(m13f): broadcast schema declarations in Welcome (parity)"
```

---

### Task 8: Client Zod mirror + drift guard (`wire.ts`, `wire.test.ts`)

**Files:**
- Modify: `src/client/core/src/wire.ts` (Zod schemas + welcome field)
- Modify: `src/client/core/src/wire.test.ts` (drift-guard type equality)

**Interfaces:**
- Consumes: generated `SchemaType`, `Schema`, `SchemaDeclaration` (`@shadowcat/types`, Task 1); `ServerMsg` welcome field (Task 7).
- Produces: `SchemaTypeSchema`, `SchemaSchema` (recursive via `z.lazy`), `SchemaDeclarationSchema`, `WireSchemaDeclaration`; `schema_declarations` on the welcome frame schema.

- [ ] **Step 1: Write the failing drift-guard test**

In `src/client/core/src/wire.test.ts`, import `SchemaDeclarationSchema` and add to the `wire drift guard — message discriminants` describe block (alongside the existing welcome-field guard):

```ts
  it("Welcome schema_declarations match ts-rs", () => {
    type W = Extract<ServerMsg, { type: "welcome" }>;
    type T = Extract<Ts.ServerMsg, { type: "welcome" }>;
    expectTypeOf<W["schema_declarations"]>().toEqualTypeOf<T["schema_declarations"]>();
  });
```

Add a new describe block for the schema types:

```ts
describe("wire drift guard — schema registry", () => {
  it("SchemaType enum", () => {
    expectTypeOf<z.infer<typeof SchemaTypeSchema>>().toEqualTypeOf<Ts.SchemaType>();
  });
  it("SchemaDeclaration shape", () => {
    // module_id/version/doc_type/subtree_pointer are strings, schema_format u32.
    expectTypeOf<
      z.infer<typeof SchemaDeclarationSchema>["subtree_pointer"]
    >().toEqualTypeOf<Ts.SchemaDeclaration["subtree_pointer"]>();
  });
});
```

Add `SchemaTypeSchema, SchemaDeclarationSchema` to the imports from `./wire`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/client/core && pnpm test wire 2>&1 | tail -25`
Expected: FAIL — `SchemaTypeSchema` / `SchemaDeclarationSchema` not exported.

- [ ] **Step 3a: Add the Zod schemas**

In `src/client/core/src/wire.ts`, after `ContractDeclarationSchema` (~line 79), add:

```ts
export const SchemaTypeSchema = z.enum([
  "object",
  "array",
  "string",
  "number",
  "boolean",
  "null",
]);

// Recursive structural type-tree (M13f tier-2). `additionalProperties` is
// `boolean | Schema`; absent fields are optional (server omits None via
// skip_serializing_if). Shape only — never a value rule.
export type WireSchema = {
  type?: z.infer<typeof SchemaTypeSchema>;
  properties?: Record<string, WireSchema>;
  required?: string[];
  additionalProperties?: boolean | WireSchema;
  items?: WireSchema;
  nullable?: boolean;
};

export const SchemaSchema: z.ZodType<WireSchema> = z.lazy(() =>
  z.object({
    type: SchemaTypeSchema.optional(),
    properties: z.record(SchemaSchema).optional(),
    required: z.array(z.string()).optional(),
    additionalProperties: z.union([z.boolean(), SchemaSchema]).optional(),
    items: SchemaSchema.optional(),
    nullable: z.boolean().optional(),
  }),
);

export const SchemaDeclarationSchema = z.object({
  module_id: z.string(),
  version: z.string(),
  schema_format: int,
  doc_type: z.string(),
  subtree_pointer: z.string(),
  schema: SchemaSchema,
});
export type WireSchemaDeclaration = z.infer<typeof SchemaDeclarationSchema>;
```

- [ ] **Step 3b: Add the welcome field**

In the `ServerMsgSchema` welcome object (~line 179, after `contract_declarations`):

```ts
    schema_declarations: z.array(SchemaDeclarationSchema),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/client/core && pnpm test wire 2>&1 | tail -25`
Expected: PASS.
Then full-repo gates: `pnpm -r typecheck 2>&1 | tail -20` and `pnpm -r test 2>&1 | tail -20`
Expected: PASS (welcome-frame fixtures across packages carry the new required field; fix any fixture the run flags by adding `schema_declarations: []`).

- [ ] **Step 5: Commit**

```bash
git add src/client/core/src/wire.ts src/client/core/src/wire.test.ts
git commit -m "feat(m13f): client Zod mirror for Schema/SchemaDeclaration + drift guard"
```

---

### Task 9: Docs + skill update (`PLAN.md`, `shadowcat-codebase-documents-permissions` skill) — reviewed skill-update gate

**Files:**
- Modify: `docs/PLAN.md` (M13f → done)
- Modify: `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md` (tier-2 seam + invariant)

**Interfaces:** none (documentation).

- [ ] **Step 1: Mark M13f done in `PLAN.md`**

Open `docs/PLAN.md`, locate the M13f roadmap entry, and move it to done/completed with the exact one-line result:

```
M13f — Server declarative schema registry (tier-2 structural validation): DONE. GM-controlled per-world SchemaDeclaration registry ((doc_type, /system/… pointer) → Schema type-tree), enforced read-only in apply_intent (Create P1 / Update P2) via validate_system_schema_tree; rejection rides the existing rejected-intent path (DataError::SchemaViolation, no new wire frame); broadcast in Welcome for parity.
```

- [ ] **Step 2: Add the tier-2 seam + invariant to the codebase skill**

Open `.claude/skills/shadowcat-codebase-documents-permissions/SKILL.md`. Add to the "Hard invariants" section:

```
- Tier-2 (M13f) validates the `system` band's SHAPE only, never values (ARCHITECTURE invariant 6): the declarable `Schema` type-tree (`type`/`properties`/`required`/`items`/`additionalProperties`/`nullable`, `additionalProperties` closed-by-default) cannot express a value rule by construction. Value legality stays tier-1 (client Zod) + fail-closed readers.
- The document writer NEVER supplies the schema that judges it: the `SchemaDeclaration` registry is GM-controlled per-world state (`/api/worlds/{id}/schemas`, `require_gm`), loaded once before the `apply_intent` transaction and enforced read-only on Create (Phase-1) / Update (Phase-2) post-images. A violation drops the transaction (per-world seq not consumed) and surfaces via `DataError::SchemaViolation` on the existing rejected-intent path — no new wire frame.
```

Add to the "Key files" / pointers section:

```
- `data/validation.rs::validate_system_schema_tree` (read-only recursive `system`-band tier-2 gate, beside `validate_engine_tree`) + `validate_value_against_schema` (pure accept/reject matcher). Types: `Schema`/`SchemaType`/`AdditionalProperties`/`SchemaDeclaration` (`data/document.rs`). Set-time gate: `routes.rs::validate_schema_declarations` (strict `/system/…` pointers, no overlap/dup, understood `schema_format`, bounds). Registry storage: `world_schema_declarations`/`set_world_schema_declarations` (world settings, key `world_schemas:{world}`). Broadcast: `Welcome.schema_declarations` (parity only).
```

- [ ] **Step 3: Commit**

```bash
git add docs/PLAN.md .claude/skills/shadowcat-codebase-documents-permissions/SKILL.md
git commit -m "docs(m13f): PLAN done + documents-permissions skill tier-2 seam/invariant"
```

- [ ] **Step 4: Reviewed skill-update gate.** Dispatch `shadowcat-spec-reviewer` on the skill diff to confirm it accurately captures the tier-2 seam, invariant, and pointers with no omission, drift, or broken pointer. This gate blocks completion at the same tier as the documentation-sync gate.

---

## Self-Review

**1. Spec coverage** (each spec section → task):
- §2 F1 (shape not values) → Task 1 types + Task 2 matcher (no value constructs). F2 (additionalProperties closed default) → Task 2 (`None | Bool(false)` reject). F3 (GM-committed per-world registry) → Task 4 storage + Task 6 GM endpoint. F4 (subtree-scoped `/system/…`) → Task 3 resolver + Task 6 pointer scoping + overlap. F5 (enforce-on-write, post-image, recursive, fail-closed) → Task 3 + Task 5 wiring. F6 (latest-wins, no retroactive) → Task 4 set-endpoint replace semantics + Task 5 uses current set (no migration code added). F7 (two versions) → `schema_format`/`version` fields (Task 1) + `SCHEMA_FORMAT_V1` gate (Task 6). F8 (set-time structural validation + ts-rs/Zod) → Task 6 + Task 8.
- §3 grammar → Task 1 (`Schema`/`SchemaType`/`AdditionalProperties`) + Task 2 semantics (nullable orthogonal to required, `{}` = any).
- §4 declaration channel/registry + set-time rules → Task 4 + Task 6 (all bullets: bounds, non-empty, unique module_id, strict `/system` descendant, no overlap/dup, understood format, well-formed tree).
- §5 enforcement (Phase-1 placement, read-only, embedded recursion, absent-ok, pre-tx load, SchemaViolation, same rejected-intent path) → Task 3 + Task 5. Phase split (Create P1 / Update P2) documented as a resolved decision.
- §6 upgrade/versioning → set-endpoint replace (Task 4) + no migration code; §6 "no retroactive" honored by not adding any corpus sweep.
- §7 composition (size cap orthogonal, deny_unknown_fields, engine out of scope, permissions unchanged) → engine untouched; no egress added.
- §8 security → GM-only endpoints (Task 6), writer-never-supplies (Task 5 pre-tx GM-registry load), two buddy-checks pre-authorized.
- §9 testing strategy → matrices (Task 2), subtree scoping/passthrough/embedded (Task 3), set-time (Task 6), rejection-shape/seq (Task 5), drift guard (Task 8). Upgrade "latest-wins" is exercised by Task 4 round-trip replace; an explicit re-set-changes-governing-schema integration test is covered by Task 5's registry-driven behavior (re-setting the registry changes what `apply_intent` reads).
- §12 seams → all mapped in the File-structure table.

**2. Placeholder scan:** No "TBD"/"add validation"/"similar to Task N". Every code step carries full code. The only intentional `/* ... */` markers are in Task 5's test scaffolding, each with an explicit instruction to copy the exact setup from a named neighboring `apply_intent_*` test — the load-bearing new code (the three `validate_system_schema_tree` call sites and the pre-tx load) is fully specified.

**3. Type consistency:** `validate_value_against_schema` / `validate_system_schema_tree` / `validate_schema` / `validate_schema_declarations` / `SchemaMismatch` / `DataError::SchemaViolation { pointer, reason }` / `SchemaDeclaration` fields (`module_id`, `version`, `schema_format`, `doc_type`, `subtree_pointer`, `schema`) / `Schema` fields (`ty`+`#[serde(rename="type")]`, `properties`, `required`, `additional_properties`+`rename="additionalProperties"`, `items`, `nullable`) / `world_schema_declarations` / `set_world_schema_declarations` / `world_schemas_key` / `Welcome.schema_declarations` are used identically across Tasks 1–9. Zod names `SchemaTypeSchema`/`SchemaSchema`/`SchemaDeclarationSchema` consistent between Task 8 wire.ts and wire.test.ts.
