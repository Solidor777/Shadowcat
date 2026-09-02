//! The conformance runner: reads the merge corpus the client package
//! generated from its own engine, runs each case through this twin, and
//! asserts exact equality. The corpus's envelope ids are deterministic
//! placeholders (`t1`, `c1`, …) that are not uuids, so the runner maps them
//! to fixed uuids at their structural positions before parsing inputs into
//! `Document`s, then normalizes its output back — known uuids to their
//! placeholders, freshly minted `restamp_subtree` ids to `restamped-N` in
//! first-seen traversal order, and `null`-valued `base`/`engine` envelope
//! keys dropped (absent and null are interchangeable to the merge; the
//! client harness applies the same normalization).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::data::document::Document;
use crate::merge::bands::MergeBase;
use crate::merge::{apply_resolutions, compute_pull, compute_revert, plan_to_update, AllVisible};

use super::is_envelope;

/// The shared corpus, read from the client package so both suites see one
/// file.
const CORPUS: &str =
    include_str!("../../../../client/core/src/__fixtures__/merge-conformance.json");

/// The corpus as parsed JSON values (the per-kind case shapes share the
/// envelope/base fields and diverge only in `expect`).
fn corpus() -> Value {
    serde_json::from_str(CORPUS).expect("merge-conformance.json parses")
}

/// Every placeholder id reachable from `doc`, recursively through
/// `embedded`: envelope `id`s, the other uuid-typed envelope fields
/// (`scope.world_id`, `parent_id`, `owner`), and `source.id` provenance
/// pointers (a template-deleted child's id appears only as a provenance
/// pointer, never as an envelope).
fn collect_envelope_ids(doc: &Value, out: &mut Vec<String>) {
    let Some(obj) = doc.as_object() else { return };
    for key in ["id", "parent_id", "owner"] {
        if let Some(s) = obj.get(key).and_then(Value::as_str) {
            out.push(s.to_string());
        }
    }
    if let Some(scope) = obj.get("scope").and_then(Value::as_object) {
        if let Some(s) = scope.get("world_id").and_then(Value::as_str) {
            out.push(s.to_string());
        }
    }
    if let Some(source) = obj.get("source").and_then(Value::as_object) {
        if let Some(s) = source.get("id").and_then(Value::as_str) {
            out.push(s.to_string());
        }
    }
    if let Some(embedded) = obj.get("embedded").and_then(Value::as_object) {
        for kids in embedded.values() {
            if let Some(arr) = kids.as_array() {
                for k in arr {
                    collect_envelope_ids(k, out);
                }
            }
        }
    }
}

/// Every `sourceId` correlation key inside a stored merge snapshot,
/// recursively (the template-deleted children of a case exist only here).
fn collect_base_ids(base: &Value, out: &mut Vec<String>) {
    let Some(obj) = base.as_object() else { return };
    if let Some(embedded) = obj.get("embedded").and_then(Value::as_object) {
        for kids in embedded.values() {
            if let Some(arr) = kids.as_array() {
                for record in arr {
                    if let Some(s) = record.get("sourceId").and_then(Value::as_str) {
                        out.push(s.to_string());
                    }
                    collect_base_ids(record, out);
                }
            }
        }
    }
}

/// Replace one string field of `obj` with its mapped uuid, when present and
/// known.
fn map_id_field(obj: &mut Map<String, Value>, key: &str, table: &BTreeMap<String, Uuid>) {
    if let Some(v) = obj.get_mut(key) {
        if let Some(s) = v.as_str() {
            if let Some(u) = table.get(s) {
                *v = Value::String(u.to_string());
            }
        }
    }
}

/// Rewrite every uuid-typed field of a document envelope (and its nested
/// envelopes) from its corpus placeholder to the mapped uuid: `id`, `scope`'s
/// `world_id`, `source.id`, `parent_id`, `owner`, and the
/// `permissions.users` keys. Payload bands are deliberately NOT walked —
/// their strings are content, not identity.
fn map_doc_ids(doc: &mut Value, table: &BTreeMap<String, Uuid>) {
    let Some(obj) = doc.as_object_mut() else {
        return;
    };
    map_id_field(obj, "id", table);
    map_id_field(obj, "parent_id", table);
    map_id_field(obj, "owner", table);
    if let Some(scope) = obj.get_mut("scope").and_then(Value::as_object_mut) {
        map_id_field(scope, "world_id", table);
    }
    if let Some(source) = obj.get_mut("source").and_then(Value::as_object_mut) {
        map_id_field(source, "id", table);
    }
    if let Some(users) = obj
        .get_mut("permissions")
        .and_then(|p| p.get_mut("users"))
        .and_then(Value::as_object_mut)
    {
        let remapped: Map<String, Value> = users
            .iter()
            .map(|(k, v)| {
                let key = table.get(k).map_or_else(|| k.clone(), |u| u.to_string());
                (key, v.clone())
            })
            .collect();
        *users = remapped;
    }
    if let Some(embedded) = obj.get_mut("embedded").and_then(Value::as_object_mut) {
        for kids in embedded.values_mut() {
            if let Some(arr) = kids.as_array_mut() {
                for k in arr {
                    map_doc_ids(k, table);
                }
            }
        }
    }
}

/// Rewrite every `sourceId` correlation key inside a stored merge snapshot,
/// recursively.
fn map_base_ids(base: &mut Value, table: &BTreeMap<String, Uuid>) {
    let Some(obj) = base.as_object_mut() else {
        return;
    };
    if let Some(embedded) = obj.get_mut("embedded").and_then(Value::as_object_mut) {
        for kids in embedded.values_mut() {
            if let Some(arr) = kids.as_array_mut() {
                for record in arr.iter_mut() {
                    if let Some(rec) = record.as_object_mut() {
                        map_id_field(rec, "sourceId", table);
                    }
                    map_base_ids(record, table);
                }
            }
        }
    }
}

/// First pass of output normalization: assign `restamped-N` placeholders to
/// every envelope id that is not a mapped input id, in first-seen traversal
/// order (the envelope's own `id` before its subtrees; object maps iterate
/// in sorted-key order, arrays positionally — the same order the client
/// harness uses).
fn assign_restamped(v: &Value, reverse: &BTreeMap<String, String>, restamped: &mut Vec<String>) {
    match v {
        Value::Array(arr) => {
            for e in arr {
                assign_restamped(e, reverse, restamped);
            }
        }
        Value::Object(obj) => {
            if is_envelope(v) {
                let id = obj
                    .get("id")
                    .and_then(Value::as_str)
                    .expect("an envelope carries a string id");
                if !reverse.contains_key(id) && !restamped.iter().any(|r| r == id) {
                    restamped.push(id.to_string());
                }
            }
            for val in obj.values() {
                assign_restamped(val, reverse, restamped);
            }
        }
        _ => {}
    }
}

/// Second pass of output normalization: rewrite mapped uuids back to their
/// corpus placeholders and restamped ids to `restamped-N` (everywhere a
/// string occurs, so a `source.id` pointing at a restamped child rewrites
/// consistently), and drop `null`-valued `base`/`engine` keys on envelopes.
fn rewrite_ids(v: &mut Value, reverse: &BTreeMap<String, String>, restamped: &[String]) {
    let envelope = is_envelope(v);
    match v {
        Value::String(s) => {
            if let Some(placeholder) = reverse.get(s) {
                *s = placeholder.clone();
            } else if let Some(i) = restamped.iter().position(|r| r == s) {
                *s = format!("restamped-{}", i + 1);
            }
        }
        Value::Array(arr) => {
            for e in arr {
                rewrite_ids(e, reverse, restamped);
            }
        }
        Value::Object(obj) => {
            if envelope {
                for key in ["base", "engine"] {
                    if obj.get(key).is_some_and(Value::is_null) {
                        obj.remove(key);
                    }
                }
            }
            for val in obj.values_mut() {
                rewrite_ids(val, reverse, restamped);
            }
        }
        _ => {}
    }
}

/// Run one corpus case through the merge engine and return the normalized
/// actual output in the case kind's `expect` shape.
fn run_case(case: &Value) -> Value {
    let name = case
        .get("name")
        .and_then(Value::as_str)
        .expect("every case has a name");
    let parent_json = case.get("parent").expect("every case has a parent");
    let child_json = case.get("child").expect("every case has a child");

    let mut ids = Vec::new();
    collect_envelope_ids(parent_json, &mut ids);
    collect_envelope_ids(child_json, &mut ids);
    if let Some(base_json) = case.get("base") {
        collect_base_ids(base_json, &mut ids);
    }
    ids.sort();
    ids.dedup();
    let forward: BTreeMap<String, Uuid> = ids
        .iter()
        .enumerate()
        .map(|(i, s)| (s.clone(), Uuid::from_u128(i as u128 + 1)))
        .collect();
    let reverse: BTreeMap<String, String> = forward
        .iter()
        .map(|(placeholder, u)| (u.to_string(), placeholder.clone()))
        .collect();

    let mut parent_v = parent_json.clone();
    map_doc_ids(&mut parent_v, &forward);
    let mut child_v = child_json.clone();
    map_doc_ids(&mut child_v, &forward);
    let parent: Document = serde_json::from_value(parent_v)
        .unwrap_or_else(|e| panic!("case '{name}': parent parses as a document: {e}"));
    let mut child: Document = serde_json::from_value(child_v)
        .unwrap_or_else(|e| panic!("case '{name}': child parses as a document: {e}"));
    child.base = match case.get("base") {
        Some(base_json) if !base_json.is_null() => {
            let mut base_v = base_json.clone();
            map_base_ids(&mut base_v, &forward);
            Some(base_v)
        }
        _ => None,
    };

    let kind = case.get("kind").and_then(Value::as_str).unwrap_or("pull");
    let actual = match kind {
        "revert" => json!({
            "update": compute_revert(&child, &parent, &AllVisible)
                .unwrap_or_else(|e| panic!("case '{name}': a corpus revert never fails: {e}"))
        }),
        "resolve" => {
            let plan = compute_pull(&child, &parent, &AllVisible)
                .unwrap_or_else(|e| panic!("case '{name}': a corpus base never fails closed: {e}"));
            let theirs: BTreeSet<String> = case
                .get("theirs")
                .and_then(Value::as_array)
                .map(|paths| {
                    paths
                        .iter()
                        .map(|p| {
                            p.as_str()
                                .expect("resolution paths are strings")
                                .to_string()
                        })
                        .collect()
                })
                .unwrap_or_default();
            let resolved = apply_resolutions(&plan.merged_bands, &plan.conflicts, &theirs);
            let update = plan_to_update(&child, &parent, &resolved);
            json!({
                "mergedBands": plan.merged_bands,
                "conflicts": plan.conflicts,
                "update": update,
            })
        }
        _ => {
            let plan = compute_pull(&child, &parent, &AllVisible)
                .unwrap_or_else(|e| panic!("case '{name}': a corpus base never fails closed: {e}"));
            json!({
                "mergedBands": plan.merged_bands,
                "conflicts": plan.conflicts,
            })
        }
    };

    let mut normalized = actual;
    let mut restamped = Vec::new();
    assign_restamped(&normalized, &reverse, &mut restamped);
    rewrite_ids(&mut normalized, &reverse, &restamped);
    normalized
}

#[test]
fn case_names_are_unique() {
    let c = corpus();
    let cases = c
        .get("cases")
        .and_then(Value::as_array)
        .expect("the corpus has a cases array");
    assert!(!cases.is_empty(), "the corpus is not empty");
    let mut names: Vec<&str> = cases
        .iter()
        .map(|case| case.get("name").and_then(Value::as_str).expect("name"))
        .collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), total, "duplicate corpus case names");
}

#[test]
fn every_case_kind_is_covered() {
    let c = corpus();
    let kinds: BTreeSet<&str> = c
        .get("cases")
        .and_then(Value::as_array)
        .expect("the corpus has a cases array")
        .iter()
        .map(|case| case.get("kind").and_then(Value::as_str).unwrap_or("pull"))
        .collect();
    assert_eq!(
        kinds,
        BTreeSet::from(["pull", "resolve", "revert"]),
        "the corpus covers pull, resolve and revert cases"
    );
}

#[test]
fn every_case_matches() {
    let c = corpus();
    for case in c
        .get("cases")
        .and_then(Value::as_array)
        .expect("the corpus has a cases array")
    {
        let name = case.get("name").and_then(Value::as_str).expect("name");
        let expected = case.get("expect").expect("every case has an expect");
        let actual = run_case(case);
        assert_eq!(
            &actual,
            expected,
            "case '{name}'\nactual:   {}\nexpected: {}",
            serde_json::to_string_pretty(&actual).expect("actual prints"),
            serde_json::to_string_pretty(expected).expect("expected prints"),
        );
    }
}

/// `MergeBase` is only referenced through `Document.base` parsing here; keep
/// the import honest by exercising the parse explicitly on one case.
#[test]
fn stored_base_parses_as_merge_base() {
    let c = corpus();
    let case = c
        .get("cases")
        .and_then(Value::as_array)
        .expect("cases")
        .iter()
        .find(|case| case.get("base").is_some_and(|b| !b.is_null()))
        .expect("at least one case carries a base");
    let base = case.get("base").expect("checked above");
    serde_json::from_value::<MergeBase>(base.clone()).expect("a stored base parses as MergeBase");
}
