//! Unit tests for the merge entry points: the fail-closed corrupt-base rule
//! and the legitimate base-less fallback on `compute_pull`, and the pieces
//! of update emission the corpus's placeholder-id normalization cannot show
//! (real-uuid `doc_id`, the unconditional `/base` refresh).

use std::collections::BTreeSet;

use serde_json::json;

use crate::data::command::{FieldChange, Operation};
use crate::merge::{
    apply_resolutions, compute_pull, compute_revert, plan_to_update, snapshot_base, AllVisible,
    MergeConflict, MergeError, ParentKind,
};

use super::{doc, source_from, test_id};

#[test]
fn compute_pull_fails_closed_on_a_corrupt_base() {
    // A stored base that is present but not a `MergeBase` shape is
    // corruption: treating it as base-less would produce a clean
    // template-wins merge that silently destroys child-local edits, so the
    // pull refuses instead.
    for corrupt in [
        json!(5),
        json!({ "name": 5 }),
        json!({ "embedded": { "items": {} } }),
    ] {
        let mut child = doc("c1");
        child.source = Some(source_from("t1"));
        child.base = Some(corrupt);
        let template = doc("t1");
        assert_eq!(
            compute_pull(&child, &template, &AllVisible),
            Err(MergeError::CorruptBase)
        );
    }
}

#[test]
fn compute_pull_falls_back_to_self_snapshot_on_an_absent_base() {
    // A base-less (never-synced) child legitimately falls back to a snapshot
    // of ITSELF: the child diff is empty against that base, so the merge is
    // clean template-wins with zero conflicts.
    let mut child = doc("c1");
    child.source = Some(source_from("t1"));
    child.name = Some("Mine".to_string());
    child.system = json!({ "hp": 99 });

    let mut template = doc("t1");
    template.name = Some("T".to_string());
    template.system = json!({ "hp": 2 });

    let plan =
        compute_pull(&child, &template, &AllVisible).expect("an absent base is not corruption");
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.merged_bands.name.as_deref(), Some("T"));
    assert_eq!(plan.merged_bands.system, json!({ "hp": 2 }));
}

#[test]
fn plan_to_update_targets_the_child_and_refreshes_base_only_when_it_changed() {
    let mut template = doc("t1");
    template.name = Some("T".to_string());
    let mut child = doc("c1");
    child.source = Some(source_from("t1"));
    child.name = Some("T".to_string());

    // No stored base: the refresh is a change (null -> snapshot).
    let plan =
        compute_pull(&child, &template, &AllVisible).expect("no base is stored on this child");
    let op = plan_to_update(&child, &template, &plan.merged_bands, true);
    let Operation::Update { doc_id, changes } = op else {
        panic!("plan_to_update emits an update");
    };
    assert_eq!(doc_id, test_id("c1"));
    assert_eq!(changes.len(), 1, "no band changed; only the base refresh");
    assert_eq!(changes[0].path, "/base");
    let snapshot = serde_json::to_value(snapshot_base(&template)).expect("snapshot serializes");
    assert_eq!(changes[0].new, snapshot);
    assert!(!changes[0].remove);

    // Stored base already equal to the template snapshot: nothing to write.
    child.base = Some(snapshot);
    let plan = compute_pull(&child, &template, &AllVisible).expect("merges");
    let Operation::Update { changes, .. } =
        plan_to_update(&child, &template, &plan.merged_bands, true)
    else {
        panic!("plan_to_update emits an update");
    };
    assert!(changes.is_empty(), "an in-sync instance yields no changes");
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
    let resolved = apply_resolutions(&bands, &conflicts, &theirs).expect("applies");
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
    // A stale snapshot (`s` moved on the template since), so the refresh is
    // a real change.
    child.base = Some(
        json!({ "name": null, "engine": { "x": 99, "hp": 5 }, "system": { "s": 0 }, "embedded": {} }),
    );

    let bands = compute_revert(&child, &template, &AllVisible).expect("reverts");
    let Operation::Update { changes, .. } = plan_to_update(&child, &template, &bands, true) else {
        panic!("plan_to_update emits an update");
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

#[test]
fn apply_resolutions_reports_an_unresolvable_take_template() {
    // The ancestor/descendant conflict shape: the child replaced `/system/obj`
    // with a scalar, the template edited `/system/obj/x`, and the conflict
    // sits at the template's path. Taking the template there has nowhere to
    // write; the refusal is an error, never a panic.
    let bands = crate::merge::MergeBands {
        name: None,
        engine: json!(null),
        system: json!({ "obj": 5 }),
        embedded: Default::default(),
    };
    let conflicts = vec![MergeConflict {
        path: "/system/obj/x".to_string(),
        base: Some(json!(1)),
        parent: Some(json!(2)),
        child: None,
        parent_kind: ParentKind::Set,
    }];
    let theirs = BTreeSet::from(["/system/obj/x".to_string()]);
    assert_eq!(
        apply_resolutions(&bands, &conflicts, &theirs),
        Err(crate::merge::PointerError::NotAContainer)
    );
}

#[test]
fn apply_resolutions_applies_sets_first_and_deletes_highest_index_first() {
    // Two template-deleted children (output indices 0 and 2) taken as
    // "theirs" plus a set inside the child at output index 3: applied in
    // report order, the first splice would shift indices 2 and 3 and the
    // later resolutions would land on the wrong children.
    let item = |n: &str, hp: i64| {
        let mut d = doc(n);
        d.system = json!({ "hp": hp });
        d
    };
    let mut embedded = std::collections::BTreeMap::new();
    embedded.insert(
        "items".to_string(),
        vec![item("i0", 0), item("i1", 1), item("i2", 2), item("i3", 3)],
    );
    let bands = crate::merge::MergeBands {
        name: None,
        engine: json!(null),
        system: json!({}),
        embedded,
    };
    let delete = |idx: usize| MergeConflict {
        path: format!("/embedded/items/{idx}"),
        base: Some(json!({ "hp": idx })),
        parent: None,
        child: Some(json!({ "hp": idx })),
        parent_kind: ParentKind::Delete,
    };
    let conflicts = vec![
        delete(0),
        delete(2),
        MergeConflict {
            path: "/embedded/items/3/system/hp".to_string(),
            base: Some(json!(3)),
            parent: Some(json!(9)),
            child: Some(json!(3)),
            parent_kind: ParentKind::Set,
        },
    ];
    let theirs: BTreeSet<String> = conflicts.iter().map(|c| c.path.clone()).collect();
    let resolved = apply_resolutions(&bands, &conflicts, &theirs).expect("applies");
    let items = &resolved.embedded["items"];
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, test_id("i1"));
    assert_eq!(items[1].id, test_id("i3"));
    assert_eq!(items[1].system, json!({ "hp": 9 }));
}

#[test]
fn apply_resolutions_deletes_a_nested_child_before_its_container() {
    // A delete inside a child that is itself deleted: deepest first, so the
    // nested delete never lands in the sibling that would take the
    // container's index after the container is spliced out.
    let mut inner = std::collections::BTreeMap::new();
    inner.insert("sub".to_string(), vec![doc("s0"), doc("s1")]);
    let mut i0 = doc("i0");
    i0.embedded = inner.clone();
    let mut i1 = doc("i1");
    i1.embedded = inner;
    let mut embedded = std::collections::BTreeMap::new();
    embedded.insert("items".to_string(), vec![i0, i1]);
    let bands = crate::merge::MergeBands {
        name: None,
        engine: json!(null),
        system: json!({}),
        embedded,
    };
    let delete = |path: &str| MergeConflict {
        path: path.to_string(),
        base: Some(json!({})),
        parent: None,
        child: Some(json!({})),
        parent_kind: ParentKind::Delete,
    };
    let conflicts = vec![
        delete("/embedded/items/0"),
        delete("/embedded/items/0/embedded/sub/1"),
    ];
    let theirs: BTreeSet<String> = conflicts.iter().map(|c| c.path.clone()).collect();
    let resolved = apply_resolutions(&bands, &conflicts, &theirs).expect("applies");
    let items = &resolved.embedded["items"];
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, test_id("i1"));
    assert_eq!(
        items[0].embedded["sub"].len(),
        2,
        "the surviving sibling is untouched"
    );
}
