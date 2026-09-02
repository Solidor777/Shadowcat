//! The per-document visibility oracle, exercised where the two index spaces
//! around an embedded merge disagree: a dropped preceding sibling on the
//! instance side, a reorder on the template side, nested depths, and both
//! sides at once. Every case would leak (or wrongly withhold) under a
//! positional hidden-pointer set compared against output-indexed conflict
//! paths; by identity the answer is index-free.

use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Value};
use uuid::Uuid;

use crate::data::document::Document;
use crate::merge::bands::EmbeddedBaseChild;
use crate::merge::{compute_pull, MergeError, MergeVisibility, Side};

use super::{doc, source_from, test_id};

/// A fake oracle keyed by document id and side.
#[derive(Default)]
struct Hide {
    /// Hidden pointers per template-side document id.
    template: HashMap<Uuid, Vec<String>>,
    /// Hidden pointers per child-side document id.
    child: HashMap<Uuid, Vec<String>>,
}

impl Hide {
    /// Hide `pointer` on the child-side document `id`.
    fn on_child(mut self, id: &str, pointer: &str) -> Self {
        self.child
            .entry(test_id(id))
            .or_default()
            .push(pointer.to_string());
        self
    }

    /// Hide `pointer` on the template-side document `id`.
    fn on_template(mut self, id: &str, pointer: &str) -> Self {
        self.template
            .entry(test_id(id))
            .or_default()
            .push(pointer.to_string());
        self
    }
}

impl MergeVisibility for Hide {
    fn hidden(&self, side: Side, doc: &Document) -> Result<Vec<String>, MergeError> {
        let table = match side {
            Side::Template => &self.template,
            Side::Child => &self.child,
        };
        Ok(table.get(&doc.id).cloned().unwrap_or_default())
    }
}

/// An oracle that cannot answer.
struct Unanswerable;

impl MergeVisibility for Unanswerable {
    fn hidden(&self, _side: Side, _doc: &Document) -> Result<Vec<String>, MergeError> {
        Err(MergeError::VisibilityUnknown)
    }
}

/// A template-side embedded child `id` with `system`.
fn template_child(id: &str, system: Value) -> Document {
    let mut d = doc(id);
    d.system = system;
    d
}

/// An instance-side embedded child `id` stamped from template child `from`
/// with `system`.
fn instance_child(id: &str, from: &str, system: Value) -> Document {
    let mut d = doc(id);
    d.source = Some(source_from(from));
    d.system = system;
    d
}

/// The base record for a child stamped from template child `from`, whose
/// bands at sync time were `system` (+ `embedded`).
fn record(from: &str, system: Value, embedded: Vec<EmbeddedBaseChild>) -> EmbeddedBaseChild {
    let mut kids = BTreeMap::new();
    if !embedded.is_empty() {
        kids.insert("sub".to_string(), embedded);
    }
    EmbeddedBaseChild {
        source_id: test_id(from).to_string(),
        name: None,
        engine: Value::Null,
        system,
        embedded: kids,
    }
}

/// A `(template, instance)` pair whose `items` collections are the given
/// children, the instance's stored base carrying `records` for `items`.
fn pair(
    template_items: Vec<Document>,
    instance_items: Vec<Document>,
    records: Vec<EmbeddedBaseChild>,
) -> (Document, Document) {
    let mut template = doc("t1");
    template
        .embedded
        .insert("items".to_string(), template_items);
    let mut child = doc("c1");
    child.source = Some(source_from("t1"));
    child.embedded.insert("items".to_string(), instance_items);
    child.base = Some(json!({
        "name": null,
        "engine": null,
        "system": {},
        "embedded": { "items": records },
    }));
    (template, child)
}

/// The conflict paths of a plan, for terse assertions.
fn paths(plan: &crate::merge::MergePlan) -> Vec<&str> {
    plan.conflicts.iter().map(|c| c.path.as_str()).collect()
}

#[test]
fn dropped_preceding_sibling_keeps_the_child_side_filter_on_the_right_child() {
    // Instance children: I_a (template deleted it, unchanged → dropped from
    // the output) then I_b, whose `/system/secret` is hidden and conflicts.
    // I_b sits at LIVE index 1 and OUTPUT index 0.
    let (template, child) = pair(
        vec![template_child("T_b", json!({ "hp": 1, "secret": "S2" }))],
        vec![
            instance_child("I_a", "T_a", json!({ "hp": 1 })),
            instance_child("I_b", "T_b", json!({ "hp": 1, "secret": "S3" })),
        ],
        vec![
            record("T_a", json!({ "hp": 1 }), vec![]),
            record("T_b", json!({ "hp": 1, "secret": "S1" }), vec![]),
        ],
    );
    let vis = Hide::default().on_child("I_b", "/system/secret");
    let plan = compute_pull(&child, &template, &vis).expect("merges");
    assert!(
        plan.conflicts.is_empty(),
        "the hidden conflict is withheld regardless of I_b's shifted index: {:?}",
        paths(&plan)
    );
    let items = &plan.merged_bands.embedded["items"];
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].system, json!({ "hp": 1, "secret": "S3" }));
}

#[test]
fn dropped_preceding_sibling_neither_leaks_nor_wrongly_withholds_across_children() {
    // I_a dropped; I_b (live 1, output 0) hides `/system/hp` and conflicts
    // on it; I_c (live 2, output 1) conflicts VISIBLY on `/system/hp`. A
    // positional set would report I_b's hidden conflict and withhold I_c's
    // visible one — the two defects at once.
    let (template, child) = pair(
        vec![
            template_child("T_b", json!({ "hp": 2 })),
            template_child("T_c", json!({ "hp": 2 })),
        ],
        vec![
            instance_child("I_a", "T_a", json!({ "hp": 1 })),
            instance_child("I_b", "T_b", json!({ "hp": 3 })),
            instance_child("I_c", "T_c", json!({ "hp": 3 })),
        ],
        vec![
            record("T_a", json!({ "hp": 1 }), vec![]),
            record("T_b", json!({ "hp": 1 }), vec![]),
            record("T_c", json!({ "hp": 1 }), vec![]),
        ],
    );
    let vis = Hide::default().on_child("I_b", "/system/hp");
    let plan = compute_pull(&child, &template, &vis).expect("merges");
    assert_eq!(paths(&plan), vec!["/embedded/items/1/system/hp"]);
    let items = &plan.merged_bands.embedded["items"];
    assert_eq!(
        items[0].system,
        json!({ "hp": 3 }),
        "I_b keeps its child-wins default"
    );
    assert_eq!(
        items[1].system,
        json!({ "hp": 3 }),
        "I_c's conflict stays pending"
    );
}

#[test]
fn template_reorder_keeps_the_template_side_filter_on_the_right_child() {
    // Template order [T_b, T_a]; instance order [I_a, I_b]. T_a's
    // `/system/secret` is hidden on the template side: T_a is template-live
    // index 1, but its correlated instance child I_a is output index 0.
    let (template, child) = pair(
        vec![
            template_child("T_b", json!({ "hp": 1, "secret": "S2" })),
            template_child("T_a", json!({ "hp": 1, "secret": "S2" })),
        ],
        vec![
            instance_child("I_a", "T_a", json!({ "hp": 1, "secret": "S3" })),
            instance_child("I_b", "T_b", json!({ "hp": 1, "secret": "S3" })),
        ],
        vec![
            record("T_a", json!({ "hp": 1, "secret": "S1" }), vec![]),
            record("T_b", json!({ "hp": 1, "secret": "S1" }), vec![]),
        ],
    );
    let vis = Hide::default().on_template("T_a", "/system/secret");
    let plan = compute_pull(&child, &template, &vis).expect("merges");
    assert_eq!(
        paths(&plan),
        vec!["/embedded/items/1/system/secret"],
        "only I_b's conflict (template side visible) is reported"
    );
    let items = &plan.merged_bands.embedded["items"];
    assert_eq!(
        items[0].system,
        json!({ "hp": 1, "secret": "S3" }),
        "I_a never receives the hidden template value"
    );
}

#[test]
fn nested_depth_resolves_visibility_by_identity_at_every_level() {
    // Top level: I_a dropped, I_b kept (output 0). Inside I_b: I_w dropped,
    // I_x kept (output 0) with a hidden `/system/secret` conflict and a
    // visible `/system/hp` conflict.
    let t_x = template_child("T_x", json!({ "hp": 2, "secret": "S2" }));
    let mut t_b = template_child("T_b", json!({}));
    t_b.embedded.insert("sub".to_string(), vec![t_x]);
    let i_x = instance_child("I_x", "T_x", json!({ "hp": 3, "secret": "S3" }));
    let i_w = instance_child("I_w", "T_w", json!({ "hp": 1 }));
    let mut i_b = instance_child("I_b", "T_b", json!({}));
    i_b.embedded.insert("sub".to_string(), vec![i_w, i_x]);
    let (template, child) = pair(
        vec![t_b],
        vec![instance_child("I_a", "T_a", json!({ "hp": 1 })), i_b],
        vec![
            record("T_a", json!({ "hp": 1 }), vec![]),
            record(
                "T_b",
                json!({}),
                vec![
                    record("T_w", json!({ "hp": 1 }), vec![]),
                    record("T_x", json!({ "hp": 1, "secret": "S1" }), vec![]),
                ],
            ),
        ],
    );
    let vis = Hide::default().on_child("I_x", "/system/secret");
    let plan = compute_pull(&child, &template, &vis).expect("merges");
    assert_eq!(
        paths(&plan),
        vec!["/embedded/items/0/embedded/sub/0/system/hp"]
    );
    let sub = &plan.merged_bands.embedded["items"][0].embedded["sub"];
    assert_eq!(sub.len(), 1);
    assert_eq!(sub[0].system, json!({ "hp": 3, "secret": "S3" }));
}

#[test]
fn both_sides_hidden_with_shifted_indices_on_both_sides() {
    // Template [T_c, T_b] (reordered); instance [I_a (dropped), I_b, I_c].
    // Child side hides I_c's `/system/mine`; template side hides T_b's
    // `/system/theirs`. Each child also has a visible `/system/hp` conflict.
    let (template, child) = pair(
        vec![
            template_child("T_c", json!({ "hp": 2, "mine": "M2", "theirs": "X2" })),
            template_child("T_b", json!({ "hp": 2, "mine": "M2", "theirs": "X2" })),
        ],
        vec![
            instance_child("I_a", "T_a", json!({ "hp": 1 })),
            instance_child(
                "I_b",
                "T_b",
                json!({ "hp": 3, "mine": "M3", "theirs": "X3" }),
            ),
            instance_child(
                "I_c",
                "T_c",
                json!({ "hp": 3, "mine": "M3", "theirs": "X3" }),
            ),
        ],
        vec![
            record("T_a", json!({ "hp": 1 }), vec![]),
            record(
                "T_b",
                json!({ "hp": 1, "mine": "M1", "theirs": "X1" }),
                vec![],
            ),
            record(
                "T_c",
                json!({ "hp": 1, "mine": "M1", "theirs": "X1" }),
                vec![],
            ),
        ],
    );
    let vis = Hide::default()
        .on_child("I_c", "/system/mine")
        .on_template("T_b", "/system/theirs");
    let plan = compute_pull(&child, &template, &vis).expect("merges");
    let mut got = paths(&plan);
    got.sort_unstable();
    assert_eq!(
        got,
        vec![
            "/embedded/items/0/system/hp",
            "/embedded/items/0/system/mine",
            "/embedded/items/1/system/hp",
            "/embedded/items/1/system/theirs",
        ]
    );
}

#[test]
fn template_deleted_conflict_is_withheld_when_the_child_hides_anything() {
    // The template deleted T_a; I_a changed since the snapshot, so it would
    // conflict as a whole-child deletion — a payload carrying the whole
    // child, withheld because I_a hides a property.
    let (template, child) = pair(
        vec![],
        vec![instance_child(
            "I_a",
            "T_a",
            json!({ "hp": 5, "secret": "S3" }),
        )],
        vec![record("T_a", json!({ "hp": 1, "secret": "S1" }), vec![])],
    );
    let vis = Hide::default().on_child("I_a", "/system/secret");
    let plan = compute_pull(&child, &template, &vis).expect("merges");
    assert!(plan.conflicts.is_empty());
    assert_eq!(
        plan.merged_bands.embedded["items"].len(),
        1,
        "the child is kept (child-wins), just unreported"
    );
}

#[test]
fn an_unanswerable_oracle_fails_the_merge_closed() {
    let (template, child) = pair(vec![], vec![], vec![]);
    assert_eq!(
        compute_pull(&child, &template, &Unanswerable).err(),
        Some(MergeError::VisibilityUnknown)
    );
}

#[test]
fn template_hidden_path_never_moves_into_the_child_in_either_direction() {
    // The template changed `/system/secret` (hidden from the requester) and
    // `/system/hp` (visible); the child is unchanged on both. Only `hp`
    // moves — a set on the hidden path is excluded, never merged, never a
    // conflict.
    let mut template = doc("t1");
    template.system = json!({ "hp": 2, "secret": "S2" });
    let mut child = doc("c1");
    child.source = Some(source_from("t1"));
    child.system = json!({ "hp": 1, "secret": "S1" });
    child.base = Some(json!({
        "name": null, "engine": null,
        "system": { "hp": 1, "secret": "S1" }, "embedded": {},
    }));
    let vis = Hide::default().on_template("t1", "/system/secret");
    let plan = compute_pull(&child, &template, &vis).expect("merges");
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.merged_bands.system, json!({ "hp": 2, "secret": "S1" }));

    // The delete direction: the requester's view of the template LACKS the
    // key (a `Within` redaction strips it), so a naive parent diff would read
    // "template deleted `secret`" and remove the child's copy.
    template.system = json!({ "hp": 2 });
    let plan = compute_pull(&child, &template, &vis).expect("merges");
    assert!(plan.conflicts.is_empty());
    assert_eq!(
        plan.merged_bands.system,
        json!({ "hp": 2, "secret": "S1" }),
        "a redaction-induced delete never reaches the child"
    );
}

#[test]
fn revert_keeps_the_child_value_on_a_template_hidden_path() {
    let mut template = doc("t1");
    template.system = json!({ "hp": 2, "secret": "S2" });
    let mut child = doc("c1");
    child.source = Some(source_from("t1"));
    child.system = json!({ "hp": 9, "secret": "S3", "extra": true });
    let vis = Hide::default().on_template("t1", "/system/secret");
    let op = crate::merge::compute_revert(&child, &template, &vis).expect("reverts");
    let crate::data::command::Operation::Update { changes, .. } = op else {
        panic!("revert emits an update");
    };
    let system = &changes
        .iter()
        .find(|c| c.path == "/system")
        .expect("system reset")
        .new;
    assert_eq!(system, &json!({ "hp": 2, "secret": "S3" }));
}
