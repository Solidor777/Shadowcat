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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
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
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default)]
    pub owner: Option<Uuid>,
    #[serde(default)]
    pub permissions: PermissionSet,
    #[serde(default)]
    pub embedded: BTreeMap<String, Vec<Document>>,
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
mod tests {
    use super::*;

    fn sample_doc() -> Document {
        Document {
            id: Uuid::from_u128(1),
            scope: Scope::World {
                world_id: Uuid::from_u128(9),
            },
            doc_type: "actor".to_string(),
            schema_version: 1,
            source: Some(Source {
                id: Uuid::from_u128(2),
                pack: Some("dnd5e".into()),
                version: 3,
            }),
            owner: Some(Uuid::from_u128(5)),
            permissions: PermissionSet::default(),
            embedded: BTreeMap::new(),
            system: serde_json::json!({ "hp": 10 }),
            created_at: 100,
            updated_at: 100,
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
}
