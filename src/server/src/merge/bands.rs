//! Merge band types and snapshot builders: the `MergeBase` stored on a
//! stamped document, the `MergeBands` a merge produces, the synthetic
//! name/engine/system tree adapters, and the placement exclusion set.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::data::document::{Document, Visibility};
use crate::data::permission::writes_a_content_band;

/// The mergeable bands of a live document; `embedded` children are full
/// documents (envelope preserved). Produced by `merge3`, written whole-band
/// by `plan_to_update`. Server-internal: it never crosses the wire (a
/// `MergeResult` carries conflicts, the committed bands ride the ordinary
/// `Event`), so it has no ts-rs export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergeBands {
    /// The document's `name` band after merge.
    pub name: Option<String>,
    /// The document's `engine` band after merge (`null` when absent).
    pub engine: Value,
    /// The document's `system` band after merge (`null` when absent).
    pub system: Value,
    /// Merged embedded collections, keyed by collection name; each child is a
    /// full document (envelope preserved), not a bands-only record.
    pub embedded: BTreeMap<String, Vec<Document>>,
}

/// One embedded child inside a `base` snapshot: bands + the `source_id`
/// correlation key (the child's `source.id` at sync time — the template
/// child's id). Recurses (finite-depth embedding). The stored JSON spells
/// the key `sourceId` (camelCase), the shape every existing snapshot was
/// written in. The serde defaults exist so a pre-validation legacy row
/// still parses on READ; at ingest `validate_engine_tree` REJECTS a record
/// with an absent key rather than letting the defaults coalesce it (a
/// coalesced record reads as unchanged against a `null` band — the
/// data-losing direction for a template-deleted child). The ts-rs export
/// is the client's `EmbeddedBaseChild`.
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
    /// The redaction policy the snapshotted child carried over its own
    /// bands at sync time (`recorded_overrides`), so egress can redact this
    /// record by the policy that governed its content (`permission`'s
    /// `own_overrides`). Spelled `propertyOverrides` on the wire like this
    /// record's other keys.
    #[serde(default)]
    pub property_overrides: BTreeMap<String, Visibility>,
}

/// The merge snapshot stored at `Document.base`: the TEMPLATE's top-level
/// bands plus recursive embedded content keyed for provenance correlation,
/// FULL and unredacted — one canonical value per instance, never relative
/// to the requester who wrote it (a requester-relative snapshot would make
/// two seats' merges rewrite each other's view forever). Written only by
/// the Create derivation (`derive_create_base`) and the merge write path
/// (`plan_to_update` under `WriteOrigin::TemplateMerge`); each recipient's
/// view of it is cut at egress by the policy it records
/// (`property_overrides`). Every field defaults so a historical record still
/// parses on READ (a missing band reads as `null`/empty, exactly the
/// coalescing the client's `snapshotBase` produces when it stamps); the
/// write path never admits such a record — `validate_engine_tree` requires
/// every key present at ingest. The ts-rs export is the client's
/// `MergeBase`.
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
    /// The redaction policy the snapshotted document carried over its own
    /// bands at sync time (`recorded_overrides`): the snapshot travels with
    /// the policy that governed its content, so egress redacts `/base` by
    /// it (`permission`'s `own_overrides`) and the client's `syncState`
    /// compares it against the template's current policy. Not consulted by
    /// the merge itself, which reduces the base by the template's CURRENT
    /// hidden set (`merge3`).
    #[serde(default)]
    pub property_overrides: BTreeMap<String, Visibility>,
}

/// The overrides of `doc` that govern its MERGEABLE bands — the pointers
/// `writes_a_content_band` admits (`/name`, `/engine…`, `/system…`), which
/// is exactly the content a `MergeBase` snapshots and a merge writes. A
/// `/base…` pointer (a policy over `doc`'s OWN snapshot of some other
/// template) is neither recorded nor propagated: it says nothing about
/// `doc`'s bands.
pub(crate) fn recorded_overrides(doc: &Document) -> BTreeMap<String, Visibility> {
    doc.permissions
        .property_overrides
        .iter()
        .filter(|(p, _)| writes_a_content_band(p))
        .map(|(p, v)| (p.clone(), *v))
        .collect()
}

/// A template's tier re-expressed for the instance that will hold it. The
/// `OwnerOrGm` tier names the OWNER of the document it sits on, and every
/// document evaluates its tiers against its own effective owner
/// (`Access::is_owner`) — so a template-owner-private value carried onto an
/// instance is private to the same person only when the two documents share
/// an effective owner (`same_owner`); otherwise it is nobody's private value
/// on the instance and becomes `GmOnly` there. `All` and `GmOnly` name no
/// owner and are unchanged. The one relation both `propagate_overrides` (the
/// instance's content policy) and `snapshot_for_instance` (the recorded
/// policy of its `/base`) apply, so the instance's content and its snapshot
/// hide the same paths from the same seats.
pub fn relate_tier(tier: Visibility, same_owner: bool) -> Visibility {
    match tier {
        Visibility::OwnerOrGm if !same_owner => Visibility::GmOnly,
        other => other,
    }
}

/// Carry `template`'s content-band policy onto `instance`, ADDITIVELY: every
/// pointer the template hides (`recorded_overrides`) gets the template's
/// tier, re-expressed for the instance (`relate_tier`), unless the instance
/// already holds a tier at least as strict (`Visibility::strictness`); an
/// instance-authored override is never removed or widened. Recurses into the
/// instance's embedded children by IDENTITY — a child whose `source.id`
/// names a template child takes that child's policy — never by position.
/// `same_owner` is the ROOT relation (an embedded child's tiers resolve
/// against the root's access, as `filter_properties` recurses). Returns
/// whether anything changed.
///
/// This is what closes the recipient direction of merge secrecy: a merge
/// write moves the template's values into the instance under the
/// requester's view, so a GM's push of a `gm_only` value lands on an
/// instance whose OWNER then reads it — unless the path arrives hidden. The
/// Create derivation and every merge write (`plan_to_update`) apply this,
/// server-authored, before the instance's content is written.
pub fn propagate_overrides(instance: &mut Document, template: &Document, same_owner: bool) -> bool {
    let mut changed = false;
    for (p, tier) in recorded_overrides(template) {
        let tier = relate_tier(tier, same_owner);
        let keep = instance
            .permissions
            .property_overrides
            .get(&p)
            .is_some_and(|own| own.strictness() >= tier.strictness());
        if !keep {
            instance.permissions.property_overrides.insert(p, tier);
            changed = true;
        }
    }
    for (coll, kids) in instance.embedded.iter_mut() {
        let Some(template_kids) = template.embedded.get(coll) else {
            continue;
        };
        for kid in kids {
            let Some(sid) = kid.source.as_ref().map(|s| s.id) else {
                continue;
            };
            if let Some(t) = template_kids.iter().find(|t| t.id == sid) {
                changed |= propagate_overrides(kid, t, same_owner);
            }
        }
    }
    changed
}

/// The `/base` value a merge write stores on an instance: `snapshot_base` of
/// the FULL template with every recorded tier re-expressed for the instance
/// (`relate_tier`, at every depth) — the policy the instance's egress
/// evaluates against its own owner. Compared and written by `plan_to_update`.
pub fn snapshot_for_instance(template: &Document, same_owner: bool) -> MergeBase {
    fn relate_records(records: &mut BTreeMap<String, Vec<EmbeddedBaseChild>>, same_owner: bool) {
        for kids in records.values_mut() {
            for k in kids {
                for tier in k.property_overrides.values_mut() {
                    *tier = relate_tier(*tier, same_owner);
                }
                relate_records(&mut k.embedded, same_owner);
            }
        }
    }
    let mut base = snapshot_base(template);
    for tier in base.property_overrides.values_mut() {
        *tier = relate_tier(*tier, same_owner);
    }
    relate_records(&mut base.embedded, same_owner);
    base
}

/// Per-`doc_type` instance-local paths that never merge. Currently only
/// `token`'s placement fields are excluded; every other doc type gets an
/// empty set. INVARIANT: equals the client's `placementExclusions` set — the
/// client's `syncState` badge excludes the same paths from its comparison,
/// so a path excluded on one side and not the other would either flag a
/// token's own position as a template change or let a merge clobber it.
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
/// descendant). The same rule as the client's `isPlacementExcluded`, which
/// `syncState` reads (see `placement_exclusions`).
pub fn is_placement_excluded(path: &str, exclusions: &[String]) -> bool {
    exclusions
        .iter()
        .any(|e| path == e || path.starts_with(&format!("{e}/")))
}

/// The three synthetic-tree bands as one object, so `merge3_tree` addresses
/// `/name`, `/engine/*`, `/system/*` at exactly the document's real pointers.
/// Absent bands coalesce to `null`.
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
/// 3-way base): the same bands minus `source_id`.
pub(crate) fn base_from_child(b: &EmbeddedBaseChild) -> MergeBase {
    MergeBase {
        name: b.name.clone(),
        engine: b.engine.clone(),
        system: b.system.clone(),
        embedded: b.embedded.clone(),
        property_overrides: b.property_overrides.clone(),
    }
}

/// `base` with every recorded policy cleared, at every depth: the CONTENT
/// of a snapshot, for a comparison that asks whether bands changed (the
/// embedded template-deleted test, `merge3_embedded`'s
/// `child_unchanged_vs_base`) and must not read a permission edit as a
/// content edit.
pub(crate) fn content_only(base: &MergeBase) -> MergeBase {
    fn clear(
        records: &BTreeMap<String, Vec<EmbeddedBaseChild>>,
    ) -> BTreeMap<String, Vec<EmbeddedBaseChild>> {
        records
            .iter()
            .map(|(coll, kids)| {
                (
                    coll.clone(),
                    kids.iter()
                        .map(|k| EmbeddedBaseChild {
                            source_id: k.source_id.clone(),
                            name: k.name.clone(),
                            engine: k.engine.clone(),
                            system: k.system.clone(),
                            embedded: clear(&k.embedded),
                            property_overrides: BTreeMap::new(),
                        })
                        .collect(),
                )
            })
            .collect()
    }
    MergeBase {
        name: base.name.clone(),
        engine: base.engine.clone(),
        system: base.system.clone(),
        embedded: clear(&base.embedded),
        property_overrides: BTreeMap::new(),
    }
}

/// Recursively reduce a document's `embedded` collections to
/// `EmbeddedBaseChild` records. The correlation key is the child's
/// `source.id` (== its template child's id); a non-provenance child falls
/// back to its own id (still a stable per-child key). The same reduction
/// the client's `snapshotBase` performs (see `snapshot_base`).
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
                    property_overrides: recorded_overrides(k),
                })
                .collect();
            (coll.clone(), records)
        })
        .collect()
}

/// Bands of a live document as a `MergeBase` (no `source_id` at the top),
/// recording the document's own content-band policy alongside
/// (`recorded_overrides`). `snapshot_base` delegates here: owned values need
/// no separate deep-cloning variant for a snapshot that outlives the call.
pub(crate) fn bands_merge_base(d: &Document) -> MergeBase {
    MergeBase {
        name: d.name.clone(),
        engine: d.engine.clone().unwrap_or(Value::Null),
        system: d.system.clone(),
        embedded: embedded_base_children(&d.embedded),
        property_overrides: recorded_overrides(d),
    }
}

/// The value stored at `Document.base` — works for both a stamped instance
/// (children keyed by their `source.id`) and a template (children key on
/// `source.id` falling back to their own id, the same correlation key its
/// instances point to). The merge write path snapshots the FULL template
/// (`plan_to_update`), policy included. INVARIANT: agrees with the client's
/// `snapshotBase` — the client's `syncState` compares its redacted view of
/// the stored base against `snapshotBase(template)` of its redacted store
/// view, and egress cuts the stored base by the policy this snapshot records
/// (`permission`'s `own_overrides`), so the two reductions must not diverge
/// or the sync badge sticks.
pub fn snapshot_base(doc: &Document) -> MergeBase {
    bands_merge_base(doc)
}

/// The Create-write `base` rule: `base` is server-owned, so the write path
/// DERIVES it rather than trusting the submitted value — a stamped instance
/// (`source` set) first takes `template`'s content-band policy
/// (`propagate_overrides` under `same_owner`, the two documents' effective-
/// owner relation, when the template is loadable), then snapshots its OWN
/// bands and policy (`snapshot_base`). The stamper's copy IS the template as
/// that seat saw it at the stamp, so this is the template snapshot the merge
/// treats as "last sync"; a hidden template value absent from a redacted
/// stamper's copy reads as template-ADDED on the first merge a seat that
/// sees it runs — the direction that lands it, hidden by the propagated
/// policy, rather than deleting it. Any other document stores no base. Any
/// client-supplied `base` is discarded, and an embedded child never carries
/// one (a submitted one is cleared recursively). `apply_intent`'s Create
/// branch calls this BEFORE validation, so the derived value is what
/// `validate_engine_tree` shape-checks and normalizes and what gets stored,
/// broadcast and logged.
pub fn derive_create_base(doc: &mut Document, template: Option<&Document>, same_owner: bool) {
    if let Some(template) = template {
        propagate_overrides(doc, template, same_owner);
    }
    let derived = doc
        .source
        .as_ref()
        .map(|_| serde_json::to_value(snapshot_base(doc)).expect("MergeBase serializes to JSON"));
    doc.base = derived;
    for children in doc.embedded.values_mut() {
        for child in children {
            clear_base_tree(child);
        }
    }
}

/// `derive_create_base`'s recursive half: an embedded child never carries
/// `base`, at any depth.
fn clear_base_tree(doc: &mut Document) {
    doc.base = None;
    for children in doc.embedded.values_mut() {
        for child in children {
            clear_base_tree(child);
        }
    }
}

/// Whether any embedded descendant of `doc` carries a `base`, at any depth —
/// the post-image shape `apply_intent`'s Update arm rejects for client
/// origins (an embedded child never carries one; see `derive_create_base`).
/// Server merge emission satisfies this by construction: `restamp_subtree`
/// and the merge's own carry-over never put a `base` on a merged child.
pub fn embedded_carries_base(doc: &Document) -> bool {
    doc.embedded
        .values()
        .flatten()
        .any(|c| c.base.is_some() || embedded_carries_base(c))
}
