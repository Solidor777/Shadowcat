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

/// Remove the object key or array element at `pointer` in `root`. No-op on
/// any missing intermediate segment. Twin of the client `deletePointer`: the
/// set-only write path cannot delete, so a merge that removes a key/element
/// rewrites the whole enclosing container (see `plan_to_update`), and this
/// builds that rewritten container in memory first.
pub(crate) fn delete_pointer(root: &mut Value, pointer: &str) {
    assert!(!pointer.is_empty(), "cannot delete the document root");
    let tokens = tokenize(pointer);
    let Some((last, intermediates)) = tokens.split_last() else {
        unreachable!("a non-empty pointer tokenizes to at least one token");
    };
    let mut cur = root;
    for tok in intermediates {
        cur = match cur {
            Value::Array(arr) => match tok.parse::<usize>().ok().and_then(|i| arr.get_mut(i)) {
                Some(v) => v,
                None => return,
            },
            Value::Object(obj) => match obj.get_mut(tok) {
                Some(v) => v,
                None => return,
            },
            _ => return,
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
/// intermediates (an explicit `null` intermediate is recreated as `{}`, the
/// client `setPointer`'s rule). Panics on a malformed pointer, an
/// out-of-range array index, or a scalar intermediate — the client throws at
/// the same points, and every path reaching this from the merge is valid by
/// construction (diffs and conflicts are generated against the tree being
/// mutated).
pub(crate) fn set_pointer(root: &mut Value, pointer: &str, value: Value) {
    assert!(
        !pointer.is_empty() && pointer.starts_with('/'),
        "invalid JSON pointer: {pointer}"
    );
    let tokens = tokenize(pointer);
    let Some((last, intermediates)) = tokens.split_last() else {
        unreachable!("a non-empty pointer tokenizes to at least one token");
    };
    let mut cur = root;
    for tok in intermediates {
        cur = match cur {
            Value::Array(arr) => {
                let i = tok
                    .parse::<usize>()
                    .ok()
                    .filter(|&i| i < arr.len())
                    .unwrap_or_else(|| panic!("cannot descend into non-container at {pointer}"));
                &mut arr[i]
            }
            Value::Object(obj) => {
                let entry = obj.entry(tok.clone()).or_insert(Value::Null);
                if entry.is_null() {
                    *entry = Value::Object(serde_json::Map::new());
                }
                entry
            }
            _ => panic!("cannot descend into non-container at {pointer}"),
        };
    }
    match cur {
        Value::Array(arr) => {
            let i = last
                .parse::<usize>()
                .ok()
                .filter(|&i| i < arr.len())
                .unwrap_or_else(|| panic!("array index out of range at {pointer}"));
            arr[i] = value;
        }
        Value::Object(obj) => {
            obj.insert(last.clone(), value);
        }
        _ => panic!("cannot descend into non-container at {pointer}"),
    }
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
fn apply_diff(root: &mut Value, d: &Diff) {
    match d {
        Diff::Set { path, value } => set_pointer(root, path, value.clone()),
        Diff::Delete { path } => delete_pointer(root, path),
    }
}

/// 3-way merge of one JSON tree (used for the `name`+`engine`+`system`
/// synthetic band tree). The merged tree starts from `child_now` and applies
/// parent-only changes; a path changed on both sides with a differing result
/// is a conflict, left at the child value ("keep mine" default). Paths in
/// `exclusions` are dropped from the parent side (never merge, never
/// conflict). Twin of the client `merge3Tree`, including its
/// ancestor/descendant overlap rule: an overlap at different depths (e.g. the
/// child deletes an object the parent edits inside) conflicts at the parent
/// change's path — the safe direction.
pub(crate) fn merge3_tree(
    base: &Value,
    parent_now: &Value,
    child_now: &Value,
    exclusions: &[String],
) -> (Value, Vec<MergeConflict>) {
    let parent_diff: Vec<Diff> = structural_diff(base, parent_now)
        .into_iter()
        .filter(|d| !is_placement_excluded(d.path(), exclusions))
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
            apply_diff(&mut merged, p);
            continue;
        }
        let exact = overlapping.iter().find(|c| c.path() == p.path());
        if let Some(e) = exact {
            if overlapping.len() == 1 && same_result(p, e) {
                continue;
            }
        }
        // The client clones `parent`/`child` out of the live source trees so a
        // conflict can never alias them; owned values make the same guarantee
        // here.
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
    (merged, conflicts)
}

/// Apply the parent's decision for a conflict into `root` (in place). Twin
/// of the client `takeTemplate`.
pub(crate) fn take_template(root: &mut Value, c: &MergeConflict) {
    match c.parent_kind {
        ParentKind::Delete => delete_pointer(root, &c.path),
        ParentKind::Set => set_pointer(
            root,
            &c.path,
            c.parent
                .clone()
                .expect("a set-kind conflict carries the parent value"),
        ),
    }
}
