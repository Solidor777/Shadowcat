//! Single-tree 3-way merge primitives: structural diffing, RFC-6901 pointer
//! helpers, and `merge3_tree` — the twin of the client engine's `merge.ts`
//! tree half. Every value is plain JSON: objects recurse key-by-key, arrays
//! are opaque leaves, scalars are leaves. Sorted-key traversal keeps the
//! output order-independent (the conformance corpus pins the order).

use std::collections::BTreeSet;

use serde_json::Value;

use crate::merge::bands::is_placement_excluded;
use crate::merge::{MergeConflict, ParentKind};

/// One structural change between two JSON trees at an RFC-6901 pointer.
/// Mirrors the client `Diff` union.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Diff {
    /// A value was written or overwritten at `path`.
    Set {
        /// The RFC-6901 pointer where the change occurred.
        path: String,
        /// The new value at `path`.
        value: Value,
    },
    /// The key/element at `path` was removed (no value).
    Delete {
        /// The RFC-6901 pointer where the removal occurred.
        path: String,
    },
}

impl Diff {
    /// The pointer this diff applies at.
    fn path(&self) -> &str {
        match self {
            Diff::Set { path, .. } | Diff::Delete { path } => path,
        }
    }
}

/// Deep structural equality: objects key-order-independent, arrays
/// positional, numbers compared as `f64` (the client engine's `===` has one
/// number type, so `1` and `1.0` are the same value), other scalars strict.
/// Twin of the client `deepEqual`.
pub(crate) fn deep_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Array(xa), Value::Array(xb)) => {
            xa.len() == xb.len() && xa.iter().zip(xb.iter()).all(|(va, vb)| deep_equal(va, vb))
        }
        (Value::Object(ma), Value::Object(mb)) => {
            ma.len() == mb.len()
                && ma
                    .iter()
                    .all(|(k, va)| mb.get(k).is_some_and(|vb| deep_equal(va, vb)))
        }
        (Value::Number(na), Value::Number(nb)) => na.as_f64() == nb.as_f64(),
        _ => a == b,
    }
}

/// RFC-6901 token escaping (`~` → `~0`, `/` → `~1`). Twin of the client
/// `escapeToken`.
pub(crate) fn escape_token(k: &str) -> String {
    k.replace('~', "~0").replace('/', "~1")
}

/// Split an RFC-6901 pointer into unescaped tokens (drops the leading empty
/// segment). Twin of the client `tokenize`.
pub(crate) fn tokenize(pointer: &str) -> Vec<String> {
    pointer
        .split('/')
        .skip(1)
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect()
}

/// Structural diff of `now` against `base` as one JSON tree. Twin of the
/// client `structuralDiff` (which defaults its `prefix` to the root).
pub(crate) fn structural_diff(base: &Value, now: &Value) -> Vec<Diff> {
    structural_diff_at(base, now, "")
}

/// The recursive body of `structural_diff`, carrying the RFC-6901 pointer
/// prefix for the subtree under consideration.
fn structural_diff_at(base: &Value, now: &Value, prefix: &str) -> Vec<Diff> {
    if let (Value::Object(base_obj), Value::Object(now_obj)) = (base, now) {
        let mut out = Vec::new();
        let keys: BTreeSet<&String> = base_obj.keys().chain(now_obj.keys()).collect();
        for key in keys {
            let p = format!("{prefix}/{}", escape_token(key));
            match (base_obj.get(key), now_obj.get(key)) {
                (Some(_), None) => out.push(Diff::Delete { path: p }),
                (None, Some(now_val)) => out.push(Diff::Set {
                    path: p,
                    value: now_val.clone(),
                }),
                (Some(base_val), Some(now_val)) => {
                    out.extend(structural_diff_at(base_val, now_val, &p));
                }
                (None, None) => unreachable!("the key came from one of the two maps"),
            }
        }
        return out;
    }
    if deep_equal(base, now) {
        return Vec::new();
    }
    vec![Diff::Set {
        path: prefix.to_string(),
        value: now.clone(),
    }]
}

/// Why an in-memory pointer write could not be applied. Every variant is
/// reachable from client input through `apply_resolutions` (a resolution
/// path names a real conflict, but the merged tree's shape at that path can
/// still refuse the template's write — the ancestor/descendant conflict
/// shape, where the child replaced a container with a scalar the template
/// edited inside), so none of them may panic: the release profile aborts on
/// panic, and one frame must never take the server down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerError {
    /// The pointer is empty or does not start with `/`.
    Malformed,
    /// An intermediate segment lands on a scalar, or names an array position
    /// that does not exist — nothing to descend into.
    NotAContainer,
    /// The terminal segment names an array position that does not exist.
    IndexOutOfRange,
    /// A `Set`-kind conflict carries no parent value to write.
    MissingValue,
    /// The written tree no longer parses as the typed structure it stands in
    /// for (embedded collections of documents).
    Unrepresentable,
}

impl std::fmt::Display for PointerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PointerError::Malformed => "malformed JSON pointer",
            PointerError::NotAContainer => "pointer descends into a non-container",
            PointerError::IndexOutOfRange => "array index out of range",
            PointerError::MissingValue => "a set-kind conflict carries no parent value",
            PointerError::Unrepresentable => "the written tree no longer parses as documents",
        })
    }
}

impl std::error::Error for PointerError {}

/// Remove the object key or array element at `pointer` in `root`. No-op on
/// any missing intermediate segment; `Malformed` on the empty pointer (the
/// document root cannot be deleted). The set-only write path cannot delete,
/// so a merge that removes a key/element rewrites the whole enclosing
/// container (see `plan_to_update`), and this builds that rewritten
/// container in memory first.
pub(crate) fn delete_pointer(root: &mut Value, pointer: &str) -> Result<(), PointerError> {
    let tokens = tokenize(pointer);
    let Some((last, intermediates)) = tokens.split_last() else {
        return Err(PointerError::Malformed);
    };
    let mut cur = root;
    for tok in intermediates {
        cur = match cur {
            Value::Array(arr) => match tok.parse::<usize>().ok().and_then(|i| arr.get_mut(i)) {
                Some(v) => v,
                None => return Ok(()),
            },
            Value::Object(obj) => match obj.get_mut(tok) {
                Some(v) => v,
                None => return Ok(()),
            },
            _ => return Ok(()),
        };
    }
    match cur {
        Value::Array(arr) => {
            if let Ok(i) = last.parse::<usize>() {
                if i < arr.len() {
                    arr.remove(i);
                }
            }
        }
        Value::Object(obj) => {
            obj.remove(last);
        }
        _ => {}
    }
    Ok(())
}

/// Read the value at `pointer`, or `None` when any segment is missing (the
/// client `getPointer` returns `undefined`, which its callers then serialize
/// as an absent key — `None` here).
pub(crate) fn get_pointer<'a>(root: &'a Value, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() {
        return Some(root);
    }
    let mut cur = root;
    for tok in tokenize(pointer) {
        cur = match cur {
            Value::Array(arr) => {
                let i = tok.parse::<usize>().ok().filter(|&i| i < arr.len())?;
                &arr[i]
            }
            Value::Object(obj) => obj.get(&tok)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Write `value` at `pointer` in `root`, creating missing object
/// intermediates (an explicit `null` intermediate is recreated as `{}`).
/// Refuses — never panics — on a malformed pointer, an out-of-range array
/// index, or a scalar intermediate. The merge's OWN applies (`apply_diff`)
/// cannot hit a refusal: a parent-only diff is applied only when no child
/// diff overlaps its path, so the child tree still holds the container
/// structure the diff was computed against. `take_template` CAN: a
/// resolution at the parent's path of an ancestor/descendant conflict
/// descends through the scalar the child wrote.
pub(crate) fn set_pointer(
    root: &mut Value,
    pointer: &str,
    value: Value,
) -> Result<(), PointerError> {
    if !pointer.starts_with('/') {
        return Err(PointerError::Malformed);
    }
    let tokens = tokenize(pointer);
    let Some((last, intermediates)) = tokens.split_last() else {
        return Err(PointerError::Malformed);
    };
    let mut cur = root;
    for tok in intermediates {
        cur = match cur {
            Value::Array(arr) => {
                let i = tok
                    .parse::<usize>()
                    .ok()
                    .filter(|&i| i < arr.len())
                    .ok_or(PointerError::NotAContainer)?;
                &mut arr[i]
            }
            Value::Object(obj) => {
                let entry = obj.entry(tok.clone()).or_insert(Value::Null);
                if entry.is_null() {
                    *entry = Value::Object(serde_json::Map::new());
                }
                entry
            }
            _ => return Err(PointerError::NotAContainer),
        };
    }
    match cur {
        Value::Array(arr) => {
            let i = last
                .parse::<usize>()
                .ok()
                .filter(|&i| i < arr.len())
                .ok_or(PointerError::IndexOutOfRange)?;
            arr[i] = value;
        }
        Value::Object(obj) => {
            obj.insert(last.clone(), value);
        }
        _ => return Err(PointerError::NotAContainer),
    }
    Ok(())
}

/// JSON-pointer subtree overlap (either contains the other, or equal). Twin
/// of the client `pathsOverlap`.
pub(crate) fn paths_overlap(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
}

/// Whether two diffs produce the same outcome (`Set` with `deep_equal`
/// values, or both `Delete`). Used by `merge3_tree` to decide a same-value
/// overlap is not a real conflict. Twin of the client `sameResult`.
fn same_result(a: &Diff, b: &Diff) -> bool {
    match (a, b) {
        (Diff::Delete { .. }, Diff::Delete { .. }) => true,
        (Diff::Set { value: va, .. }, Diff::Set { value: vb, .. }) => deep_equal(va, vb),
        _ => false,
    }
}

/// Apply one `Diff` into `root`: `Set` clones the diff's value before
/// splicing it in (the value is owned by the diff list `structural_diff`
/// produced from the source tree — cloning is the crossing point the client
/// marks with `structuredClone`), `Delete` removes the key/element via
/// `delete_pointer`. Twin of the client `applyDiff`.
fn apply_diff(root: &mut Value, d: &Diff) -> Result<(), PointerError> {
    match d {
        Diff::Set { path, value } => set_pointer(root, path, value.clone()),
        Diff::Delete { path } => delete_pointer(root, path),
    }
}

/// The requester-hidden pointers of the two documents ONE `merge3_tree` call
/// merges, each relative to that document's own root — the per-level answer
/// of the `MergeVisibility` oracle, already resolved by identity for the
/// exact template/instance pair being merged, so no index translation is
/// ever needed to compare them against this level's diff and conflict
/// paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HiddenPointers {
    /// Hidden on the template (parent) side.
    pub(crate) template: Vec<String>,
    /// Hidden on the instance (child) side.
    pub(crate) child: Vec<String>,
}

impl HiddenPointers {
    /// Whether a PARENT diff at `path` is excluded from the merge: it
    /// overlaps (equal, ancestor or descendant — the egress family's subtree
    /// predicate) a pointer hidden on the template side. The requester cannot
    /// see the template's value there, so neither a set nor a delete of it
    /// may move into the instance; the diff is dropped like a placement
    /// exclusion — never merged, never a conflict. A descendant path names
    /// hidden data directly; an ancestor path — a wholesale array — CARRIES
    /// the hidden subtree in its value.
    fn excludes_parent(&self, path: &str) -> bool {
        self.template.iter().any(|h| paths_overlap(path, h))
    }

    /// Whether a conflict at `path` is WITHHELD from the returned set: it
    /// overlaps a pointer hidden on the child side, so its `child` value (or,
    /// for an ancestor path, the subtree it carries) is something the
    /// requester may not see.
    fn withholds(&self, path: &str) -> bool {
        self.child.iter().any(|h| paths_overlap(path, h))
    }
}

/// 3-way merge of one JSON tree (used for the `name`+`engine`+`system`
/// synthetic band tree). The merged tree starts from `child_now` and applies
/// parent-only changes; a path changed on both sides with a differing result
/// is a conflict, left at the child value ("keep mine" default). Paths in
/// `exclusions` are dropped from the parent side (never merge, never
/// conflict). An overlap at different depths (e.g. the child deletes an
/// object the parent edits inside) conflicts at the parent change's path —
/// the safe direction.
///
/// `hidden` applies one rule per side. A parent diff overlapping a
/// template-hidden pointer is EXCLUDED exactly like a placement exclusion
/// (`HiddenPointers::excludes_parent`): hidden template data never moves into
/// the instance, in either direction, so no conflict can arise there either.
/// A conflict overlapping a child-hidden pointer is WITHHELD
/// (`HiddenPointers::withholds`): removed from the returned set while the
/// child-wins default it would have reported stays in the merged tree.
/// Withholding is the resolution — the caller can neither report nor resolve
/// away from the child side a conflict it never receives.
pub(crate) fn merge3_tree(
    base: &Value,
    parent_now: &Value,
    child_now: &Value,
    exclusions: &[String],
    hidden: &HiddenPointers,
) -> Result<(Value, Vec<MergeConflict>), PointerError> {
    let parent_diff: Vec<Diff> = structural_diff(base, parent_now)
        .into_iter()
        .filter(|d| {
            !is_placement_excluded(d.path(), exclusions) && !hidden.excludes_parent(d.path())
        })
        .collect();
    let child_diff = structural_diff(base, child_now);
    let mut merged = child_now.clone();
    let mut conflicts = Vec::new();
    for p in &parent_diff {
        let overlapping: Vec<&Diff> = child_diff
            .iter()
            .filter(|c| paths_overlap(c.path(), p.path()))
            .collect();
        if overlapping.is_empty() {
            apply_diff(&mut merged, p)?;
            continue;
        }
        let exact = overlapping.iter().find(|c| c.path() == p.path());
        if let Some(e) = exact {
            if overlapping.len() == 1 && same_result(p, e) {
                continue;
            }
        }
        if hidden.withholds(p.path()) {
            continue;
        }
        // Owned clones of `parent`/`child`: a conflict never aliases the live
        // source trees.
        conflicts.push(MergeConflict {
            path: p.path().to_string(),
            base: get_pointer(base, p.path()).cloned(),
            parent: match p {
                Diff::Set { value, .. } => Some(value.clone()),
                Diff::Delete { .. } => None,
            },
            child: match exact {
                Some(Diff::Set { value, .. }) => Some(value.clone()),
                _ => get_pointer(child_now, p.path()).cloned(),
            },
            parent_kind: match p {
                Diff::Set { .. } => ParentKind::Set,
                Diff::Delete { .. } => ParentKind::Delete,
            },
        });
    }
    Ok((merged, conflicts))
}

/// Apply the parent's decision for a conflict into `root` (in place). A
/// refusal means the current merged shape cannot take the template's side
/// at this path (see `set_pointer`); the caller reports it, it never aborts.
pub(crate) fn take_template(root: &mut Value, c: &MergeConflict) -> Result<(), PointerError> {
    match c.parent_kind {
        ParentKind::Delete => delete_pointer(root, &c.path),
        ParentKind::Set => set_pointer(
            root,
            &c.path,
            c.parent.clone().ok_or(PointerError::MissingValue)?,
        ),
    }
}
