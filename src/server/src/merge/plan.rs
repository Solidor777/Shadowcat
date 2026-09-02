//! The merge entry points: `merge3` over the mergeable bands (the
//! `name`+`engine`+`system` synthetic tree, plus `embedded`), the
//! pull/revert computations, and the emission of merged bands as one
//! whole-band `Operation::Update`. Twin of `merge3` in the client merge
//! engine and of the client templates module's `computePull`/
//! `computeRevert`/`planToUpdate`/`applyResolutions`/`revertBands`.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::data::command::{FieldChange, Operation};
use crate::data::document::Document;
use crate::merge::bands::{bands_tree, placement_exclusions, snapshot_base, MergeBands, MergeBase};
use crate::merge::embedded::{merge3_embedded, revert_embedded};
use crate::merge::tree::{deep_equal, merge3_tree, take_template, HiddenPointers};
use crate::merge::visibility::{MergeVisibility, Side};
use crate::merge::{MergeConflict, MergeError};

/// Result of a 3-way merge: the child-wins-default merged bands plus the
/// conflicts to resolve. Mirrors the client `MergePlan`.
#[derive(Debug, Clone, PartialEq)]
pub struct MergePlan {
    /// The merged bands (child-wins default for unresolved conflicts).
    pub merged_bands: MergeBands,
    /// Every unresolved conflict, top-level and embedded.
    pub conflicts: Vec<MergeConflict>,
}

/// The `name`/`engine`/`system` triple `revert_bands` resets and
/// `compute_revert` emits — the client `Bands` type, with absent bands
/// already coalesced to `null` values.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BandTriple {
    /// The merged `name` band.
    pub(crate) name: Option<String>,
    /// The merged `engine` band (`null` when absent).
    pub(crate) engine: Value,
    /// The merged `system` band (`null` when absent).
    pub(crate) system: Value,
}

/// Split a merged synthetic band tree back into its three bands. The tree
/// always carries all three keys (`bands_tree` builds it), so a missing key
/// reads as `null`, matching the coalescing the tree was built with.
pub(crate) fn split_bands_tree(tree: &Value) -> BandTriple {
    BandTriple {
        name: tree.get("name").and_then(Value::as_str).map(str::to_string),
        engine: tree.get("engine").cloned().unwrap_or(Value::Null),
        system: tree.get("system").cloned().unwrap_or(Value::Null),
    }
}

/// Full 3-way merge over the mergeable bands (the `name`+`engine`+`system`
/// synthetic tree, plus `embedded`). `exclusions` apply to the top-level
/// document; embedded children use their own doc_type exclusions (inside
/// `merge3_embedded`). Conflicts default to the child ("keep mine") in the
/// merged bands. Recurses into correlated embedded children via
/// `merge3_embedded`.
///
/// `vis` is asked about `parent_now` and `child_now` THEMSELVES — the two
/// documents this call merges — so the hidden pointers it returns are in
/// this level's own coordinate space, whatever depth the call sits at.
pub fn merge3(
    base: &MergeBase,
    parent_now: &Document,
    child_now: &Document,
    exclusions: &[String],
    vis: &dyn MergeVisibility,
) -> Result<MergePlan, MergeError> {
    let hidden = HiddenPointers {
        template: vis.hidden(Side::Template, parent_now)?,
        child: vis.hidden(Side::Child, child_now)?,
    };
    let (tree, tree_conflicts) = merge3_tree(
        &bands_tree(base.name.as_deref(), Some(&base.engine), Some(&base.system)),
        &bands_tree(
            parent_now.name.as_deref(),
            parent_now.engine.as_ref(),
            Some(&parent_now.system),
        ),
        &bands_tree(
            child_now.name.as_deref(),
            child_now.engine.as_ref(),
            Some(&child_now.system),
        ),
        exclusions,
        &hidden,
    );
    let bands = split_bands_tree(&tree);
    let (embedded, embedded_conflicts) = merge3_embedded(
        &base.embedded,
        &parent_now.embedded,
        &child_now.embedded,
        vis,
    )?;
    Ok(MergePlan {
        merged_bands: MergeBands {
            name: bands.name,
            engine: bands.engine,
            system: bands.system,
            embedded,
        },
        conflicts: tree_conflicts
            .into_iter()
            .chain(embedded_conflicts)
            .collect(),
    })
}

/// 3-way pull: merge the template's current state into the child, preserving
/// child-local diffs. The base is the child's stored snapshot; a base-less
/// (unstamped) child falls back to a snapshot of ITSELF, not the template —
/// the child diff is then empty against that base, so every template-side
/// change auto-applies with zero conflicts (a clean template-wins result).
///
/// A stored snapshot that is PRESENT but fails to parse as `MergeBase` is
/// corruption, not absence: returning a clean template-wins merge here would
/// silently destroy child-local edits, so the pull fails closed with
/// `MergeError::CorruptBase` and nothing is written.
pub fn compute_pull(
    child: &Document,
    template: &Document,
    vis: &dyn MergeVisibility,
) -> Result<MergePlan, MergeError> {
    let base = match &child.base {
        Some(v) => {
            serde_json::from_value::<MergeBase>(v.clone()).map_err(|_| MergeError::CorruptBase)?
        }
        None => snapshot_base(child),
    };
    merge3(
        &base,
        template,
        child,
        &placement_exclusions(&child.doc_type),
        vis,
    )
}

/// Append a `FieldChange` iff `before` and `after` structurally differ. Twin
/// of the client `pushIfChanged`.
fn push_if_changed(changes: &mut Vec<FieldChange>, path: &str, before: Value, after: Value) {
    if !deep_equal(&before, &after) {
        changes.push(FieldChange {
            path: path.to_string(),
            old: before,
            new: after,
            remove: false,
        });
    }
}

/// True when a collection value represents "no items": `null` or an empty
/// array. Used to skip a vacuous embedded-collection change even when
/// `before` and `after` differ in absence-vs-empty-array shape. Twin of the
/// client `isEmptyCollection`.
fn is_empty_collection(v: &Value) -> bool {
    v.is_null() || v.as_array().is_some_and(Vec::is_empty)
}

/// Turn merged bands into ONE `Operation::Update`: at most one whole-band
/// change per changed band (`/name`, `/engine`, `/system`), one per changed
/// embedded collection (whole array), plus a `/base` refresh whose new value
/// is the template's CURRENT snapshot (emitted unconditionally, even when it
/// equals the old one). Every `old` is the child's REAL current stored value
/// (the OCC pre-image). Whole-band/whole-collection writes are the only
/// deletion-capable form the write path accepts.
///
/// A collection key genuinely absent from `child.embedded` falls back to
/// `null` as its pre-image, NOT `[]`: the write path reads a missing JSON
/// pointer as `Value::Null`, so emitting `[]` would produce an `old` that
/// never matches the stored state and a spurious OCC rejection on an
/// otherwise honest pre-image. Twin of the client `planToUpdate`.
pub fn plan_to_update(
    child: &Document,
    template: &Document,
    merged_bands: &MergeBands,
) -> Operation {
    let mut changes = Vec::new();
    push_if_changed(
        &mut changes,
        "/name",
        child.name.as_deref().map_or(Value::Null, Value::from),
        merged_bands
            .name
            .as_deref()
            .map_or(Value::Null, Value::from),
    );
    push_if_changed(
        &mut changes,
        "/engine",
        child.engine.clone().unwrap_or(Value::Null),
        merged_bands.engine.clone(),
    );
    push_if_changed(
        &mut changes,
        "/system",
        child.system.clone(),
        merged_bands.system.clone(),
    );
    let colls: BTreeSet<&String> = child
        .embedded
        .keys()
        .chain(merged_bands.embedded.keys())
        .collect();
    for coll in colls {
        let before = child.embedded.get(coll).map_or(Value::Null, |kids| {
            serde_json::to_value(kids).expect("documents serialize to JSON")
        });
        let after = merged_bands.embedded.get(coll).map_or(Value::Null, |kids| {
            serde_json::to_value(kids).expect("documents serialize to JSON")
        });
        if !deep_equal(&before, &after)
            && !(is_empty_collection(&before) && is_empty_collection(&after))
        {
            changes.push(FieldChange {
                path: format!("/embedded/{coll}"),
                old: before,
                new: after,
                remove: false,
            });
        }
    }
    changes.push(FieldChange {
        path: "/base".to_string(),
        old: child.base.clone().unwrap_or(Value::Null),
        new: serde_json::to_value(snapshot_base(template)).expect("MergeBase serializes to JSON"),
        remove: false,
    });
    Operation::Update {
        doc_id: child.id,
        changes,
    }
}

/// Apply the user's per-field conflict choices: for each conflict whose path
/// is in `theirs`, take the template value/deletion; the rest keep the child
/// ("mine") value already in `merged_bands`. Pure (clones its input). Twin of
/// the client `applyResolutions`.
pub fn apply_resolutions(
    merged_bands: &MergeBands,
    conflicts: &[MergeConflict],
    theirs: &BTreeSet<String>,
) -> MergeBands {
    let mut root = serde_json::json!({
        "name": merged_bands.name,
        "engine": merged_bands.engine,
        "system": merged_bands.system,
        "embedded": merged_bands.embedded,
    });
    for c in conflicts {
        if theirs.contains(&c.path) {
            take_template(&mut root, c);
        }
    }
    let bands = split_bands_tree(&root);
    let embedded = serde_json::from_value(root.get("embedded").cloned().unwrap_or(Value::Null))
        .expect("merged embedded collections round-trip through JSON");
    MergeBands {
        name: bands.name,
        engine: bands.engine,
        system: bands.system,
        embedded,
    }
}

/// Reset one node's own bands to the template's current value, keeping
/// placement paths intact. Reuses `merge3_tree` with the child as its OWN
/// base (so the child diff is always empty and every parent diff
/// auto-applies with zero conflicts) — the "always take template" trick.
/// This handles only `name`/`engine`/`system`; embedded reset is the
/// SEPARATE `revert_embedded` algorithm. Twin of the client `revertBands`.
pub(crate) fn revert_bands(
    child: &Document,
    template: &Document,
    exclusions: &[String],
) -> BandTriple {
    let self_base = bands_tree(
        child.name.as_deref(),
        child.engine.as_ref(),
        Some(&child.system),
    );
    let template_now = bands_tree(
        template.name.as_deref(),
        template.engine.as_ref(),
        Some(&template.system),
    );
    let (merged, _) = merge3_tree(
        &self_base,
        &template_now,
        &self_base,
        exclusions,
        &HiddenPointers::default(),
    );
    split_bands_tree(&merged)
}

/// Revert: discard the child's local diffs on the mergeable bands — every
/// path becomes the template's current value, embedded content resets per
/// `revert_embedded` — except placement paths (kept), then refresh `base`.
/// No conflicts are possible (revert never asks the user to choose; it
/// always takes the template). Twin of the client `computeRevert`.
pub fn compute_revert(child: &Document, template: &Document) -> Operation {
    let bands = revert_bands(child, template, &placement_exclusions(&child.doc_type));
    let merged_bands = MergeBands {
        name: bands.name,
        engine: bands.engine,
        system: bands.system,
        embedded: revert_embedded(&template.embedded, &child.embedded),
    };
    plan_to_update(child, template, &merged_bands)
}
