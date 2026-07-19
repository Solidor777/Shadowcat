use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Storage/runtime scope of a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    Compendium { pack: String },
    World { world_id: Uuid },
}

/// Provenance link for the deferred pull/push merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct Source {
    pub id: Uuid,
    pub pack: Option<String>,
    pub version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum DocRole {
    Owner,
    Observer,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    All,
    GmOnly,
    /// Readable by the document's owner and the GM; redacted from everyone else.
    /// The recipient's owner-status is `Access::is_owner` (see permission.rs).
    OwnerOrGm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum WorldRole {
    Gm,
    Player,
    Spectator,
}

/// `DocRole` defaults to `None` so `PermissionSet::default()` denies access.
impl Default for DocRole {
    fn default() -> Self {
        DocRole::None
    }
}

/// Additive capability grants beyond the built-in `DocRole` floor, keyed by
/// namespaced capability string (e.g. `core:manage_embedded`). Grants widen
/// what a role/user may do on a document; they never revoke the floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct CapabilityGrants {
    #[serde(default)]
    pub by_role: BTreeMap<DocRole, BTreeSet<String>>,
    #[serde(default)]
    pub by_user: BTreeMap<Uuid, BTreeSet<String>>,
}

/// World-level capability configuration (one row per world, JSON in settings).
/// `all`/`by_type` are additive per-document grants over the `DocRole` floor,
/// doc-type-scoped. `role_caps` carries world-level capabilities keyed by
/// `WorldRole` (e.g. `core:create`) — distinct because creation has no document
/// and thus no `DocRole`. GM/admin is never keyed here; it holds every capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorldCapDefaults {
    #[serde(default)]
    pub all: CapabilityGrants,
    #[serde(default)]
    pub by_type: BTreeMap<String, CapabilityGrants>,
    #[serde(default)]
    pub role_caps: RoleCaps,
}

/// World-level capabilities keyed by `WorldRole`, doc-type-scopable. Holds the
/// `core:create` policy: a non-GM may create a document of `doc_type` only if
/// their role is granted `core:create` in `all` or `by_type[doc_type]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RoleCaps {
    #[serde(default)]
    pub all: BTreeMap<WorldRole, BTreeSet<String>>,
    #[serde(default)]
    pub by_type: BTreeMap<String, BTreeMap<WorldRole, BTreeSet<String>>>,
}

impl WorldCapDefaults {
    /// Per-document additive grants for `doc_type`: `all` unioned with
    /// `by_type[doc_type]`.
    pub fn grants_for(&self, doc_type: &str) -> CapabilityGrants {
        let mut g = self.all.clone();
        if let Some(t) = self.by_type.get(doc_type) {
            for (r, caps) in &t.by_role {
                g.by_role
                    .entry(*r)
                    .or_default()
                    .extend(caps.iter().cloned());
            }
            for (u, caps) in &t.by_user {
                g.by_user
                    .entry(*u)
                    .or_default()
                    .extend(caps.iter().cloned());
            }
        }
        g
    }

    /// Whether `role` holds world-level `cap` for `doc_type` (`role_caps`).
    pub fn role_has(&self, role: WorldRole, doc_type: &str, cap: &str) -> bool {
        self.role_caps
            .all
            .get(&role)
            .is_some_and(|s| s.contains(cap))
            || self
                .role_caps
                .by_type
                .get(doc_type)
                .and_then(|m| m.get(&role))
                .is_some_and(|s| s.contains(cap))
    }
}

/// A declarative requirement: writing any field under `path_prefix` requires the
/// actor to additionally hold every capability in `caps` (on top of the
/// structural base capability for that path). Pure data — the server enforces
/// possession and never interprets the meaning of the path or the capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct CapabilityRequirement {
    pub path_prefix: String,
    pub caps: BTreeSet<String>,
}

/// Cardinality of a UI surface contract: one provider or many.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    Singleton,
    Multi,
}

/// A UI surface contract a module provides, with its cardinality.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct ContractProvide {
    pub contract: String,
    pub cardinality: Cardinality,
}

/// A module's UI contract declaration: what surface contracts it provides and
/// which it requires an active provider for. Pure data — the server validates
/// and distributes these strings; it never holds components or runs module code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct ContractDeclaration {
    pub module_id: String,
    pub version: String,
    #[serde(default)]
    pub provides: Vec<ContractProvide>,
    #[serde(default)]
    pub requires: Vec<String>,
}

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

/// Document-level permissions: default role, per-user overrides, property-level
/// visibility keyed by JSON pointer, and additive capability grants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct PermissionSet {
    pub default: DocRole,
    pub users: BTreeMap<Uuid, DocRole>,
    pub property_overrides: BTreeMap<String, Visibility>,
    #[serde(default)]
    pub capabilities: CapabilityGrants,
    /// When `Some(role)`, a `WorldRole::Gm` actor's access to THIS document is
    /// capped like any other actor's — resolved via the same per-document
    /// `users`/role-floor logic, seeded with `role` as their fallback instead
    /// of the unconditional GM short-circuit. `None` (the default for every
    /// document type that predates this field) preserves the GM's usual
    /// unconditional `all: true` access. Lets a document (e.g. a chat whisper
    /// or a GM-only channel message) restrict even the GM unless explicitly
    /// granted — see `permission::resolve_access`.
    #[serde(default)]
    pub gm_role: Option<DocRole>,
}

/// The persisted document: typed envelope around an opaque `system` body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub id: Uuid,
    pub scope: Scope,
    pub doc_type: String,
    pub schema_version: u32,
    /// Universal display name (S2). Redacts to `null` under a `/name` override.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub source: Option<Source>,
    /// Opaque snapshot of this child's mergeable content (`name`/`engine`/`system`/
    /// `embedded`) at last sync (stamp or a successful pull/push/revert). Present only
    /// on stamped children. The server NEVER interprets it: exempt from
    /// `validate_engine_tree` (the tree walker only ever visits `engine`); will be
    /// size-capped by `validate_system_size` and writable at `/base` under
    /// `cap::WRITE_FIELDS` (see follow-up task). Client-owned shape (`MergeBase`,
    /// `@shadowcat/core`).
    #[serde(default)]
    #[ts(type = "unknown")]
    pub base: Option<serde_json::Value>,
    #[serde(default)]
    pub owner: Option<Uuid>,
    #[serde(default)]
    pub permissions: PermissionSet,
    #[serde(default)]
    pub embedded: BTreeMap<String, Vec<Document>>,
    /// Scene-entity link: the id of the scene (or other parent) this document
    /// belongs to. `None` for top-level documents (actors, compendium entries,
    /// scenes themselves). Immutable via field-path Update (envelope field).
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    /// Engine band (S1/S3): present iff `doc_type` is engine-defined; validated
    /// against the doc_type's typed struct at ingress (data/engine). Stored
    /// post-validation. `None` for community/system doc types.
    #[serde(default)]
    #[ts(type = "unknown")]
    pub engine: Option<serde_json::Value>,
    #[ts(type = "unknown")]
    pub system: serde_json::Value,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A world row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct World {
    pub id: Uuid,
    pub name: String,
    pub seq: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[cfg(test)]
pub(crate) mod tests {
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

        // S4 reservation: unknown root key `modules` is rejected
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
        // silently dropped (mirrors the TokenVisual tagged-enum hole in validation.rs).
        assert!(serde_json::from_value::<super::Schema>(serde_json::json!({
            "type": "object",
            "additionalProperties": { "type": "string", "enum": ["a"] }
        }))
        .is_err());
    }

    #[test]
    fn bad_schema_type_fails_to_deserialize() {
        assert!(
            serde_json::from_value::<super::Schema>(serde_json::json!({ "type": "integer" }))
                .is_err()
        );
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

    fn sample_doc() -> Document {
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
                "x": 0.0, "y": 0.0, "color": "#fff", "intensity": 1.0,
                "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true
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
            "message" => None, // chat's own re-root builds this doc directly; see chat/mod.rs
            "world-settings" => Some(
                serde_json::to_value(crate::data::engine::WorldSettingsEngine::default()).unwrap(),
            ),
            "vision-modes" => Some(serde_json::json!({ "modes": {} })),
            "light-gradation" => Some(serde_json::json!({ "bands": [] })),
            "chat-settings" => Some(serde_json::json!({})),
            "dice-settings" => Some(serde_json::json!({})),
            "channel-registry" => Some(serde_json::json!({ "channels": {} })),
            "faction-registry" => Some(serde_json::json!({ "factions": {} })),
            "condition-registry" => Some(serde_json::json!({ "conditions": {} })),
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
}
