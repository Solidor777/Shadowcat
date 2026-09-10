//! The merge entry points: `merge3` over the mergeable bands (the
//! `name`+`engine`+`system` synthetic tree, plus `embedded`), the
//! pull/revert computations, and the emission of merged bands as one
//! whole-band `Operation::Update`.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::data::command::{FieldChange, Operation};
use crate::data::document::{Document, OwnerStanding};
use crate::data::permission::redact_pointers;
use crate::merge::bands::{
    bands_tree, placement_exclusions, propagate_overrides, snapshot_base, MergeBands, MergeBase,
    StoredBase,
};
use crate::merge::embedded::{merge3_embedded, revert_embedded};
use crate::merge::tree::{
    deep_equal, merge3_tree, take_template, tokenize, HiddenPointers, PointerError,
};
use crate::merge::visibility::{MergeVisibility, Side};
use crate::merge::ParentKind;
use crate::merge::{MergeConflict, MergeError};

/// Result of a 3-way merge: the child-wins-default merged bands plus the
/// conflicts to resolve.
///
/// # Examples
///
/// ```
/// use shadowcat::merge::bands::MergeBands;
/// use shadowcat::merge::plan::MergePlan;
/// use std::collections::BTreeMap;
///
/// let plan = MergePlan {
///     merged_bands: MergeBands {
///         name: Some("Dragon".to_string()),
///         engine: serde_json::Value::Null,
///         system: serde_json::Value::Null,
///         embedded: BTreeMap::new(),
///     },
///     conflicts: Vec::new(),
/// };
/// assert!(plan.conflicts.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MergePlan {
    /// The merged bands (child-wins default for unresolved conflicts).
    pub merged_bands: MergeBands,
    /// Every unresolved conflict, top-level and embedded.
    pub conflicts: Vec<MergeConflict>,
}

/// The `name`/`engine`/`system` triple `revert_bands` resets and
/// `compute_revert` emits, with absent bands already coalesced to `null`
/// values.
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
///
/// The three inputs are viewed through the requester's access by ONE rule:
/// the parent is the template as `filter_properties` delivers it to the
/// requester (the caller's job), the base is the stored snapshot reduced
/// here by the SAME template-side hidden set through the same procedure
/// (`redact_pointers`), and the child stays unredacted (its own hidden set
/// withholds conflicts instead). Base and parent therefore agree on what
/// the requester may see, and no hidden template value — from either the
/// snapshot or the live template — enters the parent diff.
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, Scope};
/// use shadowcat::merge::bands::snapshot_base;
/// use shadowcat::merge::plan::merge3;
/// use shadowcat::merge::AllVisible;
/// use uuid::Uuid;
///
/// fn doc(name: &str, system: serde_json::Value) -> Document {
///     Document {
///         id: Uuid::new_v4(),
///         scope: Scope::World { world_id: Uuid::new_v4() },
///         doc_type: "actor".into(),
///         schema_version: 1,
///         name: Some(name.to_string()),
///         source: None,
///         base: None,
///         owner: None,
///         permissions: Default::default(),
///         embedded: Default::default(),
///         parent_id: None,
///         engine: None,
///         system,
///         created_at: 0,
///         updated_at: 0,
///     }
/// }
///
/// let template = doc("Dragon", serde_json::json!({ "hp": 10 }));
/// let base = snapshot_base(&template);
/// let mut parent_now = template.clone();
/// parent_now.system = serde_json::json!({ "hp": 20 }); // template changed hp
/// let child_now = doc("Dragon (mine)", serde_json::json!({ "hp": 10 })); // child untouched
///
/// let plan = merge3(&base, &parent_now, &child_now, &[], &AllVisible).unwrap();
/// assert_eq!(plan.merged_bands.system["hp"], 20); // template's change auto-applies
/// assert_eq!(plan.merged_bands.name.as_deref(), Some("Dragon (mine)")); // child-wins default
/// assert!(plan.conflicts.is_empty());
/// ```
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
    let mut base_tree = bands_tree(base.name.as_deref(), Some(&base.engine), Some(&base.system));
    redact_pointers(&mut base_tree, &hidden.template).map_err(|_| MergeError::VisibilityUnknown)?;
    let (tree, tree_conflicts) = merge3_tree(
        &base_tree,
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
    )
    .map_err(MergeError::Pointer)?;
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
/// `MergeError::CorruptBase` and nothing is written. Only the snapshot half
/// enters the merge; the recorded owner standing (`StoredBase::owner_standing`,
/// flattened alongside these same keys in the stored value) is egress's
/// business, and `MergeBase` carries no `deny_unknown_fields`, so parsing the
/// stored value straight into it ignores that key rather than requiring it —
/// a stored base predating the standing key, or a corpus fixture that never
/// carried one, is not corruption.
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, Scope};
/// use shadowcat::merge::plan::compute_pull;
/// use shadowcat::merge::AllVisible;
/// use uuid::Uuid;
///
/// fn doc(name: &str, system: serde_json::Value) -> Document {
///     Document {
///         id: Uuid::new_v4(),
///         scope: Scope::World { world_id: Uuid::new_v4() },
///         doc_type: "actor".into(),
///         schema_version: 1,
///         name: Some(name.to_string()),
///         source: None,
///         base: None,
///         owner: None,
///         permissions: Default::default(),
///         embedded: Default::default(),
///         parent_id: None,
///         engine: None,
///         system,
///         created_at: 0,
///         updated_at: 0,
///     }
/// }
///
/// let template = doc("Dragon", serde_json::json!({ "hp": 20 }));
/// let child = doc("Dragon (mine)", serde_json::json!({ "hp": 10 })); // unstamped, base = None
///
/// let plan = compute_pull(&child, &template, &AllVisible).unwrap();
/// assert_eq!(plan.merged_bands.system["hp"], 20); // clean template-wins result
/// ```
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

/// Append a `FieldChange` iff `before` and `after` structurally differ.
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
/// `before` and `after` differ in absence-vs-empty-array shape.
fn is_empty_collection(v: &Value) -> bool {
    v.is_null() || v.as_array().is_some_and(Vec::is_empty)
}

/// Turn merged bands into ONE `Operation::Update`: at most one whole-band
/// change per changed band (`/name`, `/engine`, `/system`), one per changed
/// embedded collection (whole array), a `/permissions/property_overrides`
/// change carrying the template's content-band policy propagated onto the
/// instance (`propagate_overrides`, verbatim and additive — every merged
/// embedded child takes its template child's policy the same way inside its
/// collection write), plus a `/base` refresh whose new value is `template`'s
/// CURRENT snapshot (`snapshot_base`) — `template` being the FULL, unredacted
/// template, so the stored base is one canonical value whoever wrote it —
/// under `owner_standing`, the instance owner's standing on the template as
/// the caller resolved it for THIS write (`permission::owner_standing`); each
/// emitted, like every other change, only when it differs from the stored
/// value — and a stored value that fails to parse as a `MergeBase`, or
/// carries no valid `owner_standing`, compares as differing unconditionally
/// (never as an equal-to-refreshed default), so a malformed stored base
/// always triggers the refresh that repairs it rather than comparing equal
/// to a correct one. An instance already in sync with its template
/// therefore yields an update with NO changes, which the handlers report as
/// applied without publishing (no no-op `Event` per clean instance per
/// resolution round). Every `old` is the child's REAL current stored value
/// (the OCC pre-image). Whole-band/whole-collection writes are the only
/// deletion-capable form the write path accepts.
///
/// A collection key genuinely absent from `child.embedded` falls back to
/// `null` as its pre-image, NOT `[]`: the write path reads a missing JSON
/// pointer as `Value::Null`, so emitting `[]` would produce an `old` that
/// never matches the stored state and a spurious OCC rejection on an
/// otherwise honest pre-image.
///
/// # Examples
///
/// ```
/// use shadowcat::data::command::Operation;
/// use shadowcat::data::document::{Document, OwnerStanding, Scope};
/// use shadowcat::merge::bands::MergeBands;
/// use shadowcat::merge::plan::plan_to_update;
/// use std::collections::BTreeMap;
/// use uuid::Uuid;
///
/// fn doc(name: &str, system: serde_json::Value) -> Document {
///     Document {
///         id: Uuid::new_v4(),
///         scope: Scope::World { world_id: Uuid::new_v4() },
///         doc_type: "actor".into(),
///         schema_version: 1,
///         name: Some(name.to_string()),
///         source: None,
///         base: None,
///         owner: None,
///         permissions: Default::default(),
///         embedded: Default::default(),
///         parent_id: None,
///         engine: None,
///         system,
///         created_at: 0,
///         updated_at: 0,
///     }
/// }
///
/// let template = doc("Dragon", serde_json::json!({ "hp": 20 }));
/// let child = doc("Dragon (mine)", serde_json::json!({ "hp": 10 }));
/// let merged_bands = MergeBands {
///     name: child.name.clone(),
///     engine: serde_json::Value::Null,
///     system: serde_json::json!({ "hp": 20 }),
///     embedded: BTreeMap::new(),
/// };
///
/// let op = plan_to_update(&child, &template, &merged_bands, OwnerStanding::Owner);
/// let Operation::Update { changes, .. } = op else {
///     panic!("expected an Update")
/// };
/// assert!(changes.iter().any(|c| c.path == "/system"));
/// ```
pub fn plan_to_update(
    child: &Document,
    template: &Document,
    merged_bands: &MergeBands,
    owner_standing: OwnerStanding,
) -> Operation {
    let mut changes = Vec::new();
    let mut policy_carrier = child.clone();
    policy_carrier.embedded = merged_bands.embedded.clone();
    propagate_overrides(&mut policy_carrier, template);
    let merged_bands = &MergeBands {
        name: merged_bands.name.clone(),
        engine: merged_bands.engine.clone(),
        system: merged_bands.system.clone(),
        embedded: policy_carrier.embedded,
    };
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
    push_if_changed(
        &mut changes,
        "/permissions/property_overrides",
        serde_json::to_value(&child.permissions.property_overrides)
            .expect("property overrides serialize to JSON"),
        serde_json::to_value(&policy_carrier.permissions.property_overrides)
            .expect("property overrides serialize to JSON"),
    );
    let stored_base = child.base.clone().unwrap_or(Value::Null);
    // Ingest requires `owner_standing` on every stored base
    // (`check_base_node_shape`), so an absent or unparseable value here is
    // never a legitimate row — it is exactly as much a shape violation as a
    // missing `property_overrides` key, and the read path must treat it the
    // same way: `stored_normalized` stays `Value::Null` rather than
    // defaulting the standing to the one THIS write is deriving, which would
    // make a malformed stored base compare equal to a correct one and skip
    // the refresh that would repair it.
    let stored_standing: Option<OwnerStanding> = stored_base
        .get("owner_standing")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let stored_normalized = match (
        serde_json::from_value::<MergeBase>(stored_base.clone()),
        stored_standing,
    ) {
        (Ok(snapshot), Some(standing)) => serde_json::to_value(StoredBase {
            snapshot,
            owner_standing: standing,
        })
        .expect("StoredBase serializes to JSON"),
        _ => Value::Null,
    };
    let refreshed = serde_json::to_value(StoredBase {
        snapshot: snapshot_base(template),
        owner_standing,
    })
    .expect("StoredBase serializes to JSON");
    if !deep_equal(&stored_normalized, &refreshed) {
        changes.push(FieldChange {
            path: "/base".to_string(),
            old: stored_base,
            new: refreshed,
            remove: false,
        });
    }
    Operation::Update {
        doc_id: child.id,
        changes,
    }
}

/// One RFC-6901 token as an ordering key: an array position compares
/// numerically, an object key lexically. Derived `Ord` places every `Index`
/// before every `Key`, which is irrelevant for the ordering's purpose (two
/// tokens at the same depth of one container are always the same variant).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PointerToken {
    /// A canonical array index.
    Index(usize),
    /// An object key.
    Key(String),
}

/// The ordering key of a resolution path: its tokens, array indices
/// numeric. Under `Vec`'s lexicographic `Ord` a path sorts after its own
/// prefix and `/items/10` after `/items/2`, so DESCENDING order applies the
/// deepest and highest-indexed deletes first.
fn pointer_key(path: &str) -> Vec<PointerToken> {
    tokenize(path)
        .into_iter()
        .map(|t| match t.parse::<usize>() {
            Ok(i) if i.to_string() == t => PointerToken::Index(i),
            _ => PointerToken::Key(t),
        })
        .collect()
}

/// Apply the user's per-field conflict choices: for each conflict whose path
/// is in `theirs`, take the template value/deletion; the rest keep the child
/// ("mine") value already in `merged_bands`. Pure (clones its input).
///
/// Application order is load-bearing: an array-element delete splices the
/// array and shifts every later index, and conflict paths are addressed in
/// the merged OUTPUT's index space as it was BEFORE any resolution applied.
/// So every `Set` applies first (a set never shifts anything), then every
/// `Delete` in descending `pointer_key` order — highest index first within
/// a collection, and a deeper path before the ancestor that contains it —
/// so no delete ever renumbers a path still waiting to apply.
///
/// `Err` means a chosen resolution cannot be applied to the CURRENT merged
/// shape (`take_template`'s refusal — the ancestor/descendant conflict shape
/// is the reachable case) or the resolved tree no longer parses as embedded
/// documents; nothing is written and the caller reports the set as
/// unresolvable.
///
/// # Examples
///
/// ```
/// use shadowcat::merge::bands::MergeBands;
/// use shadowcat::merge::plan::apply_resolutions;
/// use shadowcat::merge::{MergeConflict, ParentKind};
/// use std::collections::{BTreeMap, BTreeSet};
///
/// let merged_bands = MergeBands {
///     name: None,
///     engine: serde_json::Value::Null,
///     system: serde_json::json!({ "hp": 10 }), // child-wins default
///     embedded: BTreeMap::new(),
/// };
/// let conflict = MergeConflict {
///     path: "/system/hp".to_string(),
///     base: Some(serde_json::json!(5)),
///     parent: Some(serde_json::json!(20)),
///     child: Some(serde_json::json!(10)),
///     parent_kind: ParentKind::Set,
/// };
/// let theirs: BTreeSet<String> = ["/system/hp".to_string()].into_iter().collect();
///
/// let resolved = apply_resolutions(&merged_bands, &[conflict], &theirs).unwrap();
/// assert_eq!(resolved.system["hp"], 20); // "theirs" took the template's value
/// ```
pub fn apply_resolutions(
    merged_bands: &MergeBands,
    conflicts: &[MergeConflict],
    theirs: &BTreeSet<String>,
) -> Result<MergeBands, PointerError> {
    let mut root = serde_json::json!({
        "name": merged_bands.name,
        "engine": merged_bands.engine,
        "system": merged_bands.system,
        "embedded": merged_bands.embedded,
    });
    let chosen: Vec<&MergeConflict> = conflicts
        .iter()
        .filter(|c| theirs.contains(&c.path))
        .collect();
    for c in chosen.iter().filter(|c| c.parent_kind == ParentKind::Set) {
        take_template(&mut root, c)?;
    }
    let mut deletes: Vec<&MergeConflict> = chosen
        .iter()
        .copied()
        .filter(|c| c.parent_kind == ParentKind::Delete)
        .collect();
    deletes.sort_by_cached_key(|c| std::cmp::Reverse(pointer_key(&c.path)));
    for c in deletes {
        take_template(&mut root, c)?;
    }
    let bands = split_bands_tree(&root);
    let embedded = serde_json::from_value(root.get("embedded").cloned().unwrap_or(Value::Null))
        .map_err(|_| PointerError::Unrepresentable)?;
    Ok(MergeBands {
        name: bands.name,
        engine: bands.engine,
        system: bands.system,
        embedded,
    })
}

/// Reset one node's own bands to the template's current value, keeping
/// placement paths intact. Reuses `merge3_tree` with the child as its OWN
/// base (so the child diff is always empty and every parent diff
/// auto-applies with zero conflicts) — the "always take template" trick.
/// This handles only `name`/`engine`/`system`; embedded reset is the
/// SEPARATE `revert_embedded` algorithm.
pub(crate) fn revert_bands(
    child: &Document,
    template: &Document,
    exclusions: &[String],
    hidden_template: Vec<String>,
) -> Result<BandTriple, PointerError> {
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
    let hidden = HiddenPointers {
        template: hidden_template,
        child: Vec::new(),
    };
    let (merged, _) = merge3_tree(&self_base, &template_now, &self_base, exclusions, &hidden)?;
    Ok(split_bands_tree(&merged))
}

/// Revert: discard the child's local diffs on the mergeable bands — every
/// path becomes the template's current value, embedded content resets per
/// `revert_embedded` — except placement paths (kept) and paths hidden from
/// the requester on the template side (kept: the requester cannot see the
/// template's value there, so the child's own stays). `template` is the
/// requester-VISIBLE template; the caller emits the result through
/// `plan_to_update` against the full template, which refreshes `base`. No
/// conflicts are possible (revert never asks the user to choose; it always
/// takes the template).
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, Scope};
/// use shadowcat::merge::plan::compute_revert;
/// use shadowcat::merge::AllVisible;
/// use uuid::Uuid;
///
/// fn doc(name: &str, system: serde_json::Value) -> Document {
///     Document {
///         id: Uuid::new_v4(),
///         scope: Scope::World { world_id: Uuid::new_v4() },
///         doc_type: "actor".into(),
///         schema_version: 1,
///         name: Some(name.to_string()),
///         source: None,
///         base: None,
///         owner: None,
///         permissions: Default::default(),
///         embedded: Default::default(),
///         parent_id: None,
///         engine: None,
///         system,
///         created_at: 0,
///         updated_at: 0,
///     }
/// }
///
/// let template = doc("Dragon", serde_json::json!({ "hp": 20 }));
/// let child = doc("Dragon (mine)", serde_json::json!({ "hp": 10 }));
///
/// let bands = compute_revert(&child, &template, &AllVisible).unwrap();
/// assert_eq!(bands.system["hp"], 20); // child-local edit discarded
/// assert_eq!(bands.name.as_deref(), Some("Dragon")); // template's name wins too
/// ```
pub fn compute_revert(
    child: &Document,
    template: &Document,
    vis: &dyn MergeVisibility,
) -> Result<MergeBands, MergeError> {
    let bands = revert_bands(
        child,
        template,
        &placement_exclusions(&child.doc_type),
        vis.hidden(Side::Template, template)?,
    )
    .map_err(MergeError::Pointer)?;
    Ok(MergeBands {
        name: bands.name,
        engine: bands.engine,
        system: bands.system,
        embedded: revert_embedded(&template.embedded, &child.embedded, vis)?,
    })
}
