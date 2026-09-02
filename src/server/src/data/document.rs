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

/// The actor copy a token document embeds under its `embedded["actor"]` list,
/// if any — the ONE extraction shared by combat evaluation
/// (`combat::eval::formula_host`/`effect_host_doc`) and chat's token-instance
/// roll host (`chat::host`), so the token→copy step cannot fork between the
/// combatant walk and the roll binding.
pub(crate) fn embedded_actor_copy(token: &Document) -> Option<&Document> {
    token.embedded.get("actor").and_then(|v| v.first())
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
pub(crate) mod tests;
