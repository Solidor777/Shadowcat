//! Embedded-collection merge and revert: `merge3_embedded` correlates
//! instance and template children by `source.id` (never by index) against
//! the `base` snapshot's membership records; `revert_embedded` resets
//! collections against the CURRENT template with no snapshot consulted.
//! Twin of the embedded half of the client merge engine.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use uuid::Uuid;

use crate::data::document::{Document, Source};
use crate::merge::bands::{
    bands_merge_base, base_from_child, placement_exclusions, EmbeddedBaseChild, MergeBands,
};
use crate::merge::plan::{merge3, revert_bands};
use crate::merge::tree::structural_diff;
use crate::merge::{MergeConflict, ParentKind};

/// Whether an instance child's bands are unchanged versus its base record.
/// Twin of the client `childUnchangedVsBase`.
fn child_unchanged_vs_base(child: &Document, b: &EmbeddedBaseChild) -> bool {
    let before = serde_json::to_value(base_from_child(b)).expect("MergeBase serializes to JSON");
    let after =
        serde_json::to_value(bands_merge_base(child)).expect("MergeBase serializes to JSON");
    structural_diff(&before, &after).is_empty()
}

/// Write merged bands into a fresh clone of `child`'s envelope (its
/// `permissions`/`scope`/`source`/`owner` etc. are preserved — the client
/// marks this as a full-clone crossing point so the result never aliases the
/// live instance). A `null` engine band normalizes to an absent engine band;
/// both serialize identically on the envelope. Twin of the client
/// `applyMergedBands`.
fn apply_merged_bands(child: &Document, bands: &MergeBands) -> Document {
    let mut out = child.clone();
    out.name = bands.name.clone();
    out.engine = if bands.engine.is_null() {
        None
    } else {
        Some(bands.engine.clone())
    };
    out.system = bands.system.clone();
    out.embedded = bands.embedded.clone();
    out
}

/// Rewrite each conflict's `path` to be relative to the top-level document,
/// prefixing it with the embedded child's collection + index
/// (`/embedded/<coll>/<idx>`). `idx` is the child's index within the OUTPUT
/// array being built, not its position in the instance's collection. Twin of
/// the client `prefixConflicts`.
fn prefix_conflicts(conflicts: Vec<MergeConflict>, coll: &str, idx: usize) -> Vec<MergeConflict> {
    let prefix = format!("/embedded/{coll}/{idx}");
    conflicts
        .into_iter()
        .map(|mut c| {
            c.path = format!("{prefix}{}", c.path);
            c
        })
        .collect()
}

/// 3-way merge of the embedded collections, correlating instance↔template
/// children by `source.id`↔`id`, using `base.embedded[coll][*].source_id` as
/// the membership record. Twin of the client `merge3Embedded`: pass 1 walks
/// the instance children in order (instance-added kept, correlated children
/// recursed or fail-safe kept, template-deleted children dropped when
/// unchanged and conflicted when changed); pass 2 restamps template-added
/// children in. Collection order is sorted, instance order is preserved
/// within a collection (a template-side reorder does not reorder the
/// instance's children), and template additions append after them.
pub(crate) fn merge3_embedded(
    base: &BTreeMap<String, Vec<EmbeddedBaseChild>>,
    parent_embedded: &BTreeMap<String, Vec<Document>>,
    child_embedded: &BTreeMap<String, Vec<Document>>,
) -> (BTreeMap<String, Vec<Document>>, Vec<MergeConflict>) {
    let mut merged = BTreeMap::new();
    let mut conflicts = Vec::new();
    let colls: BTreeSet<&String> = base
        .keys()
        .chain(parent_embedded.keys())
        .chain(child_embedded.keys())
        .collect();
    for coll in colls {
        let base_kids: &[EmbeddedBaseChild] = base.get(coll).map_or(&[], Vec::as_slice);
        let parent_kids: &[Document] = parent_embedded.get(coll).map_or(&[], Vec::as_slice);
        let child_kids: &[Document] = child_embedded.get(coll).map_or(&[], Vec::as_slice);
        let template_by_id: HashMap<Uuid, &Document> =
            parent_kids.iter().map(|t| (t.id, t)).collect();
        let base_by_source: HashMap<&str, &EmbeddedBaseChild> = base_kids
            .iter()
            .map(|b| (b.source_id.as_str(), b))
            .collect();
        let mut out: Vec<Document> = Vec::new();

        // Pass 1: walk instance children in order.
        for cd in child_kids {
            let sid = cd.source.as_ref().map(|s| s.id);
            let correlated = sid.is_some_and(|id| {
                template_by_id.contains_key(&id)
                    || base_by_source.contains_key(id.to_string().as_str())
            });
            if !correlated {
                // Instance-added → keep.
                out.push(cd.clone());
                continue;
            }
            let t = sid.and_then(|id| template_by_id.get(&id).copied());
            let b = sid.and_then(|id| base_by_source.get(id.to_string().as_str()).copied());
            if let Some(t) = t {
                if let Some(b) = b {
                    let idx = out.len();
                    let plan = merge3(
                        &base_from_child(b),
                        t,
                        cd,
                        &placement_exclusions(&cd.doc_type),
                    );
                    out.push(apply_merged_bands(cd, &plan.merged_bands));
                    conflicts.extend(prefix_conflicts(plan.conflicts, coll, idx));
                } else {
                    // Base-missing matched → keep instance (fail-safe; no
                    // 3-way base).
                    out.push(cd.clone());
                }
                continue;
            }
            // Template absent, base present → the template deleted this
            // correlation. `correlated && t.is_none()` forces `b` to be
            // `Some` (correlation's disjunction requires the base side when
            // the template side is absent).
            if let Some(b) = b {
                if child_unchanged_vs_base(cd, b) {
                    continue; // drop
                }
                let idx = out.len();
                // Kept pending resolution.
                out.push(cd.clone());
                conflicts.push(MergeConflict {
                    path: format!("/embedded/{coll}/{idx}"),
                    base: Some(b.system.clone()),
                    parent: None,
                    child: Some(cd.system.clone()),
                    parent_kind: ParentKind::Delete,
                });
            }
        }

        // Pass 2: template-added children (in template, absent from base, no
        // instance copy).
        for t in parent_kids {
            if base_by_source.contains_key(t.id.to_string().as_str()) {
                continue;
            }
            if child_kids
                .iter()
                .any(|cd| cd.source.as_ref().is_some_and(|s| s.id == t.id))
            {
                continue;
            }
            out.push(restamp_subtree(t));
        }

        merged.insert(coll.clone(), out);
    }
    (merged, conflicts)
}

/// Deep-clone `doc` into a new subtree: fresh `id`, `source` pointing at the
/// template (`doc.id`), recursively for every embedded child. Used to stamp
/// a template-added embedded child into an instance. A restamped subtree has
/// no prior sync snapshot of its own, so `base` is cleared — the client
/// clears the same field, and a top-level stamped document's snapshot is set
/// explicitly after the whole tree is assembled. Twin of the client
/// `restampSubtree`.
pub(crate) fn restamp_subtree(doc: &Document) -> Document {
    let mut out = doc.clone();
    out.id = Uuid::new_v4();
    out.source = Some(Source {
        id: doc.id,
        pack: None,
        version: doc.source.as_ref().map_or(1, |s| s.version),
    });
    out.base = None;
    out.embedded = doc
        .embedded
        .iter()
        .map(|(coll, kids)| {
            (
                coll.clone(),
                kids.iter().map(restamp_subtree).collect::<Vec<_>>(),
            )
        })
        .collect();
    out
}

/// Embedded-collection reset for revert. `merge3_embedded` KEEPS an
/// uncorrelated instance child ("instance-added" — correct when preserving
/// local additions is the point); revert wants the opposite: discard every
/// local addition. Correlation is by `child.source.id` against a CURRENT
/// template child's id — no stored `base` is consulted (there is nothing to
/// preserve), so a correlated child is recurse-reset (`revert_child`), an
/// uncorrelated child is DROPPED, and a template child with no correlating
/// instance child is freshly stamped in. Twin of the client
/// `revertEmbedded`.
pub(crate) fn revert_embedded(
    parent_embedded: &BTreeMap<String, Vec<Document>>,
    child_embedded: &BTreeMap<String, Vec<Document>>,
) -> BTreeMap<String, Vec<Document>> {
    let mut merged = BTreeMap::new();
    let colls: BTreeSet<&String> = parent_embedded
        .keys()
        .chain(child_embedded.keys())
        .collect();
    for coll in colls {
        let parent_kids: &[Document] = parent_embedded.get(coll).map_or(&[], Vec::as_slice);
        let child_kids: &[Document] = child_embedded.get(coll).map_or(&[], Vec::as_slice);
        let template_by_id: HashMap<Uuid, &Document> =
            parent_kids.iter().map(|t| (t.id, t)).collect();
        let mut out: Vec<Document> = Vec::new();
        for cd in child_kids {
            let t = cd
                .source
                .as_ref()
                .and_then(|s| template_by_id.get(&s.id).copied());
            if let Some(t) = t {
                out.push(revert_child(cd, t));
            }
            // else: no correlation → child-added → dropped.
        }
        for t in parent_kids {
            if !child_kids
                .iter()
                .any(|cd| cd.source.as_ref().is_some_and(|s| s.id == t.id))
            {
                out.push(restamp_subtree(t));
            }
        }
        merged.insert(coll.clone(), out);
    }
    merged
}

/// Reset one matched embedded child: its own bands to the template
/// counterpart (placement kept), recursing into its own embedded collections
/// the same way. The returned envelope is a full clone of `child`
/// (`permissions`/`scope`/`source`/`owner` preserved — the same crossing
/// point `apply_merged_bands` marks). Twin of the client `revertChild`.
pub(crate) fn revert_child(child: &Document, template: &Document) -> Document {
    let bands = revert_bands(child, template, &placement_exclusions(&child.doc_type));
    let mut out = child.clone();
    out.name = bands.name;
    out.engine = if bands.engine.is_null() {
        None
    } else {
        Some(bands.engine)
    };
    out.system = bands.system;
    out.embedded = revert_embedded(&template.embedded, &child.embedded);
    out
}
