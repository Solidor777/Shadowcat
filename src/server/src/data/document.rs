// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Storage/runtime scope of a document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// World-independent library content, keyed by pack name.
    Compendium {
        /// The compendium pack this document ships in.
        pack: String,
    },
    /// Live game content owned by exactly one world.
    World {
        /// The owning world's id; cross-world reads compare against this
        /// (`world_of` is the single chokepoint).
        world_id: Uuid,
    },
}

/// The world a document belongs to, or `None` for a compendium document (no world).
/// Single chokepoint for "which world does this doc scope to" — callers that need cross-world
/// pinning (a doc referenced from one world's context must belong to THAT world) compare this
/// against the caller's own `world_id` rather than re-matching `Scope` inline.
///
/// # Examples
///
/// ```text
/// world_of(&scene_doc) == Some(world_id)   // Scope::World
/// world_of(&pack_doc)  == None             // Scope::Compendium
/// ```
pub(crate) fn world_of(doc: &Document) -> Option<Uuid> {
    match doc.scope {
        Scope::World { world_id } => Some(world_id),
        Scope::Compendium { .. } => None,
    }
}

/// Provenance link for the deferred pull/push merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct Source {
    /// The template/original document this one was stamped from.
    pub id: Uuid,
    /// Compendium pack of the source, when it came from one.
    pub pack: Option<String>,
    /// Source content version at stamp time (provenance for pull/push).
    pub version: u32,
}

/// Per-document access tier. Derived `Ord` follows declaration order —
/// `Owner < Observer < None` — so SMALLER is STRONGER; strengthening code uses
/// `.min()` (see `effective_role`'s token owner floor). A
/// `PermissionSet::users` entry REPLACES the default for that user (it can
/// demote as well as promote), not a max.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum DocRole {
    /// Full read/write on the document (subject to capability requirements).
    Owner,
    /// Read-only; property overrides may still hide fields.
    Observer,
    /// No access: the document is invisible to this user.
    None,
}

/// Property-level visibility for a JSON-pointer subtree (`PermissionSet::
/// property_overrides`). Enforced per recipient by `Access::can_see` inside
/// `filter_properties` — hidden values are stripped BEFORE transmission, never
/// sent-then-hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Visible to every recipient who can see the document at all.
    All,
    /// Visible only to recipients whose `Access` carries GM sight.
    GmOnly,
    /// Readable by the document's owner and the GM; redacted from everyone else.
    /// The recipient's owner-status is `Access::is_owner`.
    OwnerOrGm,
}

/// Per-world membership role (orthogonal to the server admin/user tier).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum WorldRole {
    /// Runs the world: sees everything (unless a doc sets `gm_role`), holds
    /// every capability, manages scenes/modules/invites.
    Gm,
    /// Plays in the world under document permissions and capability grants.
    Player,
    /// Read-only presence; no write capabilities.
    Spectator,
}

/// `DocRole` defaults to `None` so `PermissionSet::default()` denies access.
impl Default for DocRole {
    fn default() -> Self {
        DocRole::None
    }
}

/// Additive capability grants beyond the built-in `DocRole` floor. Each map is
/// keyed by grantee — a `DocRole` or a user id — and its values are namespaced
/// capability strings (e.g. `core:manage_embedded`). Grants widen what a
/// role/user may do on a document; they never revoke the floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct CapabilityGrants {
    /// Extra capabilities granted to everyone holding a given `DocRole`.
    #[serde(default)]
    pub by_role: BTreeMap<DocRole, BTreeSet<String>>,
    /// Extra capabilities granted to specific users, regardless of role.
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
    /// Per-document grants applied to every doc_type.
    #[serde(default)]
    pub all: CapabilityGrants,
    /// Per-document grants applied only to the keyed doc_type (unioned with `all`).
    #[serde(default)]
    pub by_type: BTreeMap<String, CapabilityGrants>,
    /// World-level (documentless) capabilities keyed by `WorldRole` — e.g. `core:create`.
    #[serde(default)]
    pub role_caps: RoleCaps,
}

/// World-level capabilities keyed by `WorldRole`, doc-type-scopable. Holds the
/// `core:create` policy: a non-GM may create a document of `doc_type` only if
/// their role is granted `core:create` in `all` or `by_type[doc_type]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RoleCaps {
    /// World-level capabilities per role, for every doc_type.
    #[serde(default)]
    pub all: BTreeMap<WorldRole, BTreeSet<String>>,
    /// World-level capabilities per role, scoped to the keyed doc_type.
    #[serde(default)]
    pub by_type: BTreeMap<String, BTreeMap<WorldRole, BTreeSet<String>>>,
}

impl WorldCapDefaults {
    /// Per-document additive grants for `doc_type`: `all` unioned with
    /// `by_type[doc_type]`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::document::{DocRole, WorldCapDefaults};
    ///
    /// let mut defaults = WorldCapDefaults::default();
    /// defaults
    ///     .all
    ///     .by_role
    ///     .entry(DocRole::Observer)
    ///     .or_default()
    ///     .insert("core:manage_embedded".into());
    ///
    /// let g = defaults.grants_for("actor");
    /// assert!(g.by_role[&DocRole::Observer].contains("core:manage_embedded"));
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::document::{WorldCapDefaults, WorldRole};
    ///
    /// let mut defaults = WorldCapDefaults::default();
    /// defaults
    ///     .role_caps
    ///     .by_type
    ///     .entry("drawing".into())
    ///     .or_default()
    ///     .entry(WorldRole::Player)
    ///     .or_default()
    ///     .insert("core:create".into());
    ///
    /// assert!(defaults.role_has(WorldRole::Player, "drawing", "core:create"));
    /// assert!(!defaults.role_has(WorldRole::Player, "actor", "core:create"));
    /// ```
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
    /// JSON-pointer prefix the rule applies to (writes at or under it).
    pub path_prefix: String,
    /// Capabilities the writer must ALL hold, on top of the structural base
    /// capability for the path (`required_cap_for_path`).
    pub caps: BTreeSet<String>,
}

/// Cardinality of a UI surface contract: one provider or many.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    /// Exactly one active provider renders the surface.
    Singleton,
    /// Any number of providers contribute side by side.
    Multi,
}

/// A UI surface contract a module provides, with its cardinality.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct ContractProvide {
    /// The surface contract id (e.g. `shadowcat.panel`).
    pub contract: String,
    /// How many providers the contract admits.
    pub cardinality: Cardinality,
}

/// A module's UI contract declaration: what surface contracts it provides and
/// which it requires an active provider for. Pure data — the server validates
/// and distributes these strings; it never holds components or runs module code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct ContractDeclaration {
    /// Declaring module's id.
    pub module_id: String,
    /// Declaring module's version.
    pub version: String,
    /// Contracts this module provides, with cardinality.
    #[serde(default)]
    pub provides: Vec<ContractProvide>,
    /// Contract ids this module requires an active provider for.
    #[serde(default)]
    pub requires: Vec<String>,
}

/// A single JSON type tag for a schema node. Shape only — never a value
/// discriminator, keeping schema validation built from this type structural
/// rather than semantic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum SchemaType {
    /// A JSON object.
    Object,
    /// A JSON array.
    Array,
    /// A JSON string.
    String,
    /// A JSON number (integer or float — shape only, no bounds).
    Number,
    /// A JSON boolean.
    Boolean,
    /// JSON `null`.
    Null,
}

/// `additionalProperties`: a bool (`false` = closed, `true` = any) or a subschema
/// every non-`properties` key must match. Serialized untagged (`boolean | Schema`);
/// the hand-written `Deserialize` routes a JSON object straight into `Schema` via
/// `MapAccessDeserializer` so the inner schema's `deny_unknown_fields` is enforced
/// (an untagged/internally-tagged derive would buffer through `Content` and drop
/// that check — the same serde limitation documented for `TokenVisual`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum AdditionalProperties {
    /// `false` = closed object; `true` = any extra keys allowed.
    Bool(bool),
    /// Every non-`properties` key must match this subschema.
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

/// A structural (shape-only) type-tree node. By construction cannot
/// express a value rule (no enum/bounds/pattern/combinators), so a schema built
/// from this type can only ever check shape, never a value.
/// `deny_unknown_fields` makes a malformed schema fail to
/// deserialize at the set endpoint. An all-absent node (`{}`) matches any JSON.
/// Cross-field legality (e.g. `items` only on an array) is not enforced by serde;
/// `validate_schema` enforces it at set-time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(deny_unknown_fields)]
pub struct Schema {
    /// Required JSON type of the node; absent = any type.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ty: Option<SchemaType>,
    /// Per-key subschemas for an object node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub properties: Option<BTreeMap<String, Schema>>,
    /// Object keys that must be present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub required: Option<Vec<String>>,
    #[serde(
        rename = "additionalProperties",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    /// Policy for object keys not named in `properties`; absent behaves as
    /// `false` (closed — a deliberate, documented divergence from JSON Schema).
    #[ts(optional, type = "boolean | Schema")]
    pub additional_properties: Option<AdditionalProperties>,
    /// Subschema every array element must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub items: Option<Box<Schema>>,
    /// When true, JSON `null` also matches regardless of `type`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub nullable: Option<bool>,
}

/// A module's per-`(doc_type, subtree)` structural schema. Pure
/// data — the server stores and interprets it as a shape check, never as code.
/// `subtree_pointer` is a strict `/system/…` descendant (enforced at set-time).
/// `schema_format` is the engine-owned vocabulary version; `version` is the
/// module's content version (provenance only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(deny_unknown_fields)]
pub struct SchemaDeclaration {
    /// Declaring module's id (provenance).
    pub module_id: String,
    /// Declaring module's content version (provenance only).
    pub version: String,
    /// Engine-owned schema-vocabulary version (`SCHEMA_FORMAT_V1`).
    pub schema_format: u32,
    /// The doc_type whose `system` band this schema constrains.
    pub doc_type: String,
    /// Strict `/system/…` descendant pointer the schema roots at (set-time enforced).
    pub subtree_pointer: String,
    /// The structural type-tree itself.
    pub schema: Schema,
}

/// Document-level permissions: default role, per-user overrides, property-level
/// visibility keyed by JSON pointer, and additive capability grants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct PermissionSet {
    /// Role floor for any user without a `users` entry. `DocRole::None` here
    /// makes the document invisible by default (fail-closed).
    pub default: DocRole,
    /// Per-user role that REPLACES `default` for that user — it can demote as
    /// well as promote (`effective_role`).
    pub users: BTreeMap<Uuid, DocRole>,
    /// Per-JSON-pointer visibility tiers; enforced per recipient by
    /// `Access::can_see` inside `filter_properties` before transmission.
    pub property_overrides: BTreeMap<String, Visibility>,
    /// Additive capability grants beyond the role floor (never revoking it).
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
    /// Stable identity; immutable after create.
    pub id: Uuid,
    /// World or compendium the document lives in.
    pub scope: Scope,
    /// Free-form type string; engine-defined types additionally carry `engine`.
    pub doc_type: String,
    /// Envelope schema version for forward migration.
    pub schema_version: u32,
    /// Universal display name. Redacts to `null` under a `/name` override.
    #[serde(default)]
    pub name: Option<String>,
    /// Provenance of a stamped instance; `None` = not an instance. Immutable:
    /// `required_cap_for_path` maps no capability to `/source`, so no write
    /// path can re-target a document at a different template.
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
    /// Owning user. On tokens, ownership is EFFECTIVE — the token's own owner,
    /// else the linked actor's (`effective_owner`); on every
    /// other doc_type this is provenance only and grants no capability.
    #[serde(default)]
    pub owner: Option<Uuid>,
    /// Per-document access policy (see `PermissionSet`).
    #[serde(default)]
    pub permissions: PermissionSet,
    /// Child documents keyed by collection name (e.g. an actor's `item`s),
    /// recursively full `Document`s.
    #[serde(default)]
    pub embedded: BTreeMap<String, Vec<Document>>,
    /// Scene-entity link: the id of the scene (or other parent) this document
    /// belongs to. `None` for top-level documents (actors, compendium entries,
    /// scenes themselves). Immutable via field-path Update (envelope field).
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    /// Engine band: present iff `doc_type` is engine-defined; validated
    /// against the doc_type's typed struct at ingress (data/engine). Stored
    /// post-validation. `None` for community/system doc types.
    #[serde(default)]
    #[ts(type = "unknown")]
    pub engine: Option<serde_json::Value>,
    /// Opaque game-system band: structurally validated only (size, JSON,
    /// optional tier-2 shape schema) — the server NEVER interprets its meaning.
    #[ts(type = "unknown")]
    pub system: serde_json::Value,
    /// Creation time, Unix epoch milliseconds.
    pub created_at: i64,
    /// Last-write time, Unix epoch milliseconds.
    pub updated_at: i64,
}

/// A world row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct World {
    /// Stable world identity.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Monotonic event sequence number — the world's broadcast watermark.
    pub seq: i64,
    /// Creation time, Unix epoch milliseconds.
    pub created_at: i64,
    /// Last-write time, Unix epoch milliseconds.
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
            "message" => None, // chat's own re-root builds this doc directly; see `chat::build_message_doc`
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
