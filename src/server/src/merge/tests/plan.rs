//! Unit tests for the merge entry points: the base fallback a stored
//! snapshot that cannot parse takes, and the pieces of update emission the
//! corpus's placeholder-id normalization cannot show (real-uuid `doc_id`,
//! the unconditional `/base` refresh).

use std::collections::BTreeSet;

use serde_json::json;

use crate::data::command::{FieldChange, Operation};
use crate::merge::{
    apply_resolutions, compute_pull, compute_revert, plan_to_update, snapshot_base, MergeConflict,
    ParentKind,
};

use super::{doc, source_from, test_id};

#[test]
fn compute_pull_falls_back_to_self_snapshot_on_an_unparsable_base() {
    // A stored base that is not even a `MergeBase` shape cannot carry
    // correlation information the merge could trust; the child is treated as
    // base-less, so the merge is clean template-wins with zero conflicts.
    let mut child = doc("c1");
    child.source = Some(source_from("t1"));
    child.name = Some("Mine".to_string());
    child.system = json!({ "hp": 99 });
    child.base = Some(json!(5));

    let mut template = doc("t1");
    template.name = Some("T".to_string());
    template.system = json!({ "hp": 2 });

    let plan = compute_pull(&child, &template);
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.merged_bands.name.as_deref(), Some("T"));
    assert_eq!(plan.merged_bands.system, json!({ "hp": 2 }));
}

#[test]
fn plan_to_update_targets_the_child_and_always_refreshes_base() {
    let mut template = doc("t1");
    template.name = Some("T".to_string());
    let mut child = doc("c1");
    child.source = Some(source_from("t1"));
    child.name = Some("T".to_string());

    let plan = compute_pull(&child, &template);
    let op = plan_to_update(&child, &template, &plan.merged_bands);
    let Operation::Update { doc_id, changes } = op else {
        panic!("plan_to_update emits an update");
    };
    assert_eq!(doc_id, test_id("c1"));
    // Nothing but the unconditional `/base` refresh: no band changed.
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, "/base");
    assert_eq!(
        changes[0].new,
        serde_json::to_value(snapshot_base(&template)).expect("snapshot serializes")
    );
    assert!(!changes[0].remove);
}

#[test]
fn apply_resolutions_takes_the_template_value_for_their_paths_only() {
    let bands = crate::merge::MergeBands {
        name: None,
        engine: json!(null),
        system: json!({ "a": "mine", "b": "mine" }),
        embedded: Default::default(),
    };
    let conflicts = vec![
        MergeConflict {
            path: "/system/a".to_string(),
            base: Some(json!("x")),
            parent: Some(json!("theirs")),
            child: Some(json!("mine")),
            parent_kind: ParentKind::Set,
        },
        MergeConflict {
            path: "/system/b".to_string(),
            base: Some(json!("x")),
            parent: Some(json!("theirs")),
            child: Some(json!("mine")),
            parent_kind: ParentKind::Set,
        },
    ];
    let theirs = BTreeSet::from(["/system/a".to_string()]);
    let resolved = apply_resolutions(&bands, &conflicts, &theirs);
    assert_eq!(resolved.system, json!({ "a": "theirs", "b": "mine" }));
    // The input bands are untouched (the client engine is pure here too).
    assert_eq!(bands.system, json!({ "a": "mine", "b": "mine" }));
}

#[test]
fn compute_revert_keeps_token_placement_and_refreshes_base() {
    let mut template = doc("t1");
    template.doc_type = "token".to_string();
    template.engine = Some(json!({ "x": 99, "hp": 5 }));
    template.system = json!({ "s": 1 });

    let mut child = doc("c1");
    child.doc_type = "token".to_string();
    child.source = Some(source_from("t1"));
    child.engine = Some(json!({ "x": 3, "hp": 8 }));
    child.system = json!({ "s": 2, "extra": true });
    child.base = Some(
        json!({ "name": null, "engine": { "x": 99, "hp": 5 }, "system": { "s": 1 }, "embedded": {} }),
    );

    let op = compute_revert(&child, &template);
    let Operation::Update { changes, .. } = op else {
        panic!("compute_revert emits an update");
    };
    let find = |path: &str| {
        changes
            .iter()
            .find(|c| c.path == path)
            .expect("change present")
    };
    assert_eq!(find("/engine").new, json!({ "x": 3, "hp": 5 }));
    assert_eq!(find("/system").new, json!({ "s": 1 }));
    assert_eq!(find("/base").old, child.base.clone().expect("base present"));
    // Revert never conflicts and never emits removals — whole-band writes only.
    assert!(changes.iter().all(|c: &FieldChange| !c.remove));
}
