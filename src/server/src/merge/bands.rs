//! Merge band types and snapshot builders: the `MergeBase` stored on a
//! stamped document, the `MergeBands` a merge produces, the synthetic
//! name/engine/system tree adapters, and the placement exclusion set. Twin
//! of the band half of the client merge engine.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::data::document::Document;

/// The mergeable bands of a live document; `embedded` children are full
/// documents (envelope preserved). Produced by `merge3`, written whole-band
/// by `plan_to_update`. Mirrors the client `MergeBands`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct MergeBands {
    /// The document's `name` band after merge.
    pub name: Option<String>,
    /// The document's `engine` band after merge (`null` when absent).
    #[ts(type = "unknown")]
    pub engine: Value,
    /// The document's `system` band after merge (`null` when absent).
    #[ts(type = "unknown")]
    pub system: Value,
    /// Merged embedded collections, keyed by collection name; each child is a
    /// full document (envelope preserved), not a bands-only record.
    pub embedded: BTreeMap<String, Vec<Document>>,
}

/// One embedded child inside a `base` snapshot: bands + the `source_id`
/// correlation key (the child's `source.id` at sync time — the template
/// child's id). Recurses (finite-depth embedding). The stored JSON spells
/// the key `sourceId` (camelCase), the shape every existing snapshot was
/// written in. Mirrors the client `EmbeddedBaseChild`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedBaseChild {
    /// The child's `source.id` at sync time — the correlation key
    /// `merge3_embedded` matches instance/template children by.
    pub source_id: String,
    /// The child's `name` band at sync time.
    #[serde(default)]
    pub name: Option<String>,
    /// The child's `engine` band at sync time (`null` when absent).
    #[serde(default)]
    #[ts(type = "unknown")]
    pub engine: Value,
    /// The child's `system` band at sync time (`null` when absent).
    #[serde(default)]
    #[ts(type = "unknown")]
    pub system: Value,
    /// The child's own embedded collections at sync time, recursively in the
    /// same shape.
    #[serde(default)]
    pub embedded: BTreeMap<String, Vec<EmbeddedBaseChild>>,
}

/// The merge snapshot stored at `Document.base`: top-level bands plus
/// recursive embedded content keyed for provenance correlation. Every field
/// defaults so a partial or historical record still parses (a missing band
/// reads as `null`/empty, exactly what the client engine's `?? null`
/// coalescing produces). Mirrors the client `MergeBase`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct MergeBase {
    /// The document's `name` band at sync time.
    #[serde(default)]
    pub name: Option<String>,
    /// The document's `engine` band at sync time (`null` when absent).
    #[serde(default)]
    #[ts(type = "unknown")]
    pub engine: Value,
    /// The document's `system` band at sync time (`null` when absent).
    #[serde(default)]
    #[ts(type = "unknown")]
    pub system: Value,
    /// Embedded collections at sync time, keyed by collection name, each
    /// reduced to `EmbeddedBaseChild` records (not full documents).
    #[serde(default)]
    pub embedded: BTreeMap<String, Vec<EmbeddedBaseChild>>,
}

/// Per-`doc_type` instance-local paths that never merge. Currently only
/// `token`'s placement fields are excluded; every other doc type gets an
/// empty set. Twin of the client `placementExclusions`.
pub fn placement_exclusions(doc_type: &str) -> Vec<String> {
    if doc_type == "token" {
        vec![
            "/engine/x".to_string(),
            "/engine/y".to_string(),
            "/engine/rotation".to_string(),
        ]
    } else {
        Vec::new()
    }
}

/// Whether `path` is inside the placement exclusion set (equal or a
/// descendant). Twin of the client `isPlacementExcluded`.
pub fn is_placement_excluded(path: &str, exclusions: &[String]) -> bool {
    exclusions
        .iter()
        .any(|e| path == e || path.starts_with(&format!("{e}/")))
}

/// The three synthetic-tree bands as one object, so `merge3_tree` addresses
/// `/name`, `/engine/*`, `/system/*` at exactly the document's real pointers.
/// Absent bands coalesce to `null` (the client `bandsTree`'s `?? null`).
pub(crate) fn bands_tree(
    name: Option<&str>,
    engine: Option<&Value>,
    system: Option<&Value>,
) -> Value {
    serde_json::json!({
        "name": name,
        "engine": engine.cloned().unwrap_or(Value::Null),
        "system": system.cloned().unwrap_or(Value::Null),
    })
}

/// `MergeBase`-shaped bands of an embedded base child (for the recursive
/// 3-way base): the same bands minus `source_id`. Twin of the client
/// `baseFromChild`.
pub(crate) fn base_from_child(b: &EmbeddedBaseChild) -> MergeBase {
    MergeBase {
        name: b.name.clone(),
        engine: b.engine.clone(),
        system: b.system.clone(),
        embedded: b.embedded.clone(),
    }
}

/// Recursively reduce a document's `embedded` collections to
/// `EmbeddedBaseChild` records. The correlation key is the child's
/// `source.id` (== its template child's id); a non-provenance child falls
/// back to its own id (still a stable per-child key). Twin of the recursion
/// the client `snapshotEmbedded`/`bandsMergeBase` share.
fn embedded_base_children(
    embedded: &BTreeMap<String, Vec<Document>>,
) -> BTreeMap<String, Vec<EmbeddedBaseChild>> {
    embedded
        .iter()
        .map(|(coll, kids)| {
            let records = kids
                .iter()
                .map(|k| EmbeddedBaseChild {
                    source_id: k
                        .source
                        .as_ref()
                        .map(|s| s.id.to_string())
                        .unwrap_or_else(|| k.id.to_string()),
                    name: k.name.clone(),
                    engine: k.engine.clone().unwrap_or(Value::Null),
                    system: k.system.clone(),
                    embedded: embedded_base_children(&k.embedded),
                })
                .collect();
            (coll.clone(), records)
        })
        .collect()
}

/// Bands of a live document as a `MergeBase` (no `source_id` at the top).
/// Twin of the client `bandsMergeBase`. The client keeps a separate
/// deep-cloning `snapshotBase` for values that outlive the call; owned Rust
/// values make that distinction a no-op, so `snapshot_base` delegates here.
pub(crate) fn bands_merge_base(d: &Document) -> MergeBase {
    MergeBase {
        name: d.name.clone(),
        engine: d.engine.clone().unwrap_or(Value::Null),
        system: d.system.clone(),
        embedded: embedded_base_children(&d.embedded),
    }
}

/// The value stored at `Document.base` — works for both a stamped instance
/// (children keyed by their `source.id`) and a template (children key on
/// `source.id` falling back to their own id, the same correlation key its
/// instances point to). Twin of the client `snapshotBase`.
pub fn snapshot_base(doc: &Document) -> MergeBase {
    bands_merge_base(doc)
}
