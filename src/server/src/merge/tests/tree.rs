//! Unit tests for the single-tree primitives (the corpus covers their
//! behaviour inside full merges; these pin the edges the corpus cannot
//! express, like `f64` number equality across JSON's int/float spellings).

use serde_json::json;

use crate::merge::tree::{
    deep_equal, delete_pointer, escape_token, get_pointer, paths_overlap, set_pointer,
    structural_diff, take_template, tokenize, PointerError,
};
use crate::merge::{MergeConflict, ParentKind};

#[test]
fn deep_equal_compares_numbers_as_f64() {
    // JSON has one number type on the client; `1` and `1.0` are the same
    // value there, so the twin compares numerically, not by representation.
    assert!(deep_equal(&json!(1), &json!(1.0)));
    assert!(deep_equal(&json!({ "a": 1 }), &json!({ "a": 1.0 })));
    assert!(!deep_equal(&json!(1), &json!(2)));
}

#[test]
fn deep_equal_is_key_order_independent_and_positional() {
    assert!(deep_equal(
        &json!({ "a": 1, "b": 2 }),
        &json!({ "b": 2, "a": 1 })
    ));
    assert!(!deep_equal(&json!([1, 2]), &json!([2, 1])));
    assert!(!deep_equal(&json!(0), &json!(false)));
    assert!(deep_equal(&json!(null), &json!(null)));
}

#[test]
fn structural_diff_emits_sorted_escaped_pointers() {
    let diffs = structural_diff(&json!({}), &json!({ "b/x": 1, "a~y": 2 }));
    let paths: Vec<&str> = diffs
        .iter()
        .map(|d| match d {
            crate::merge::tree::Diff::Set { path, .. } => path.as_str(),
            crate::merge::tree::Diff::Delete { path } => path.as_str(),
        })
        .collect();
    assert_eq!(paths, ["/a~0y", "/b~1x"]);
}

#[test]
fn structural_diff_treats_arrays_as_opaque_leaves() {
    let diffs = structural_diff(&json!({ "a": [1, 2] }), &json!({ "a": [1, 2, 3] }));
    assert_eq!(
        diffs,
        [crate::merge::tree::Diff::Set {
            path: "/a".to_string(),
            value: json!([1, 2, 3]),
        }]
    );
}

#[test]
fn structural_diff_emits_deletes_for_dropped_keys() {
    let diffs = structural_diff(&json!({ "a": 1, "b": 2 }), &json!({ "a": 1 }));
    assert_eq!(
        diffs,
        [crate::merge::tree::Diff::Delete {
            path: "/b".to_string()
        }]
    );
}

#[test]
fn escape_and_tokenize_round_trip() {
    assert_eq!(escape_token("a/b~c"), "a~1b~0c");
    assert_eq!(tokenize("/a~1b~0c/x"), ["a/b~c", "x"]);
}

#[test]
fn delete_pointer_removes_keys_and_splices_elements() {
    let mut obj = json!({ "a": { "b": 1, "c": 2 } });
    delete_pointer(&mut obj, "/a/b").expect("deletes");
    assert_eq!(obj, json!({ "a": { "c": 2 } }));

    let mut arr = json!({ "xs": [10, 20, 30] });
    delete_pointer(&mut arr, "/xs/1").expect("deletes");
    assert_eq!(arr, json!({ "xs": [10, 30] }));

    let mut missing = json!({ "a": 1 });
    delete_pointer(&mut missing, "/b/c").expect("no-ops");
    assert_eq!(missing, json!({ "a": 1 }));
}

#[test]
fn set_pointer_creates_missing_intermediates_and_recreates_null_ones() {
    let mut root = json!({ "a": null });
    set_pointer(&mut root, "/a/b/c", json!(7)).expect("sets");
    assert_eq!(root, json!({ "a": { "b": { "c": 7 } } }));

    let mut arr = json!({ "xs": [1, 2] });
    set_pointer(&mut arr, "/xs/0", json!(9)).expect("sets");
    assert_eq!(arr, json!({ "xs": [9, 2] }));
}

#[test]
fn get_pointer_reads_nested_values_and_misses_cleanly() {
    let root = json!({ "a": { "b": [10, 20] } });
    assert_eq!(get_pointer(&root, "/a/b/1"), Some(&json!(20)));
    assert_eq!(get_pointer(&root, ""), Some(&root));
    assert_eq!(get_pointer(&root, "/a/b/9"), None);
    assert_eq!(get_pointer(&root, "/a/nope/x"), None);
}

#[test]
fn paths_overlap_covers_equal_and_ancestor_pairs_only() {
    assert!(paths_overlap("/a", "/a"));
    assert!(paths_overlap("/a/b", "/a"));
    assert!(paths_overlap("/a", "/a/b"));
    assert!(!paths_overlap("/a/b", "/a/bc"));
    assert!(!paths_overlap("/a", "/b"));
}

#[test]
fn take_template_applies_set_and_delete() {
    let mut root = json!({ "a": 3, "b": 5 });
    take_template(
        &mut root,
        &MergeConflict {
            path: "/a".to_string(),
            base: Some(json!(1)),
            parent: Some(json!(2)),
            child: Some(json!(3)),
            parent_kind: ParentKind::Set,
        },
    )
    .expect("takes the template value");
    assert_eq!(root, json!({ "a": 2, "b": 5 }));
    take_template(
        &mut root,
        &MergeConflict {
            path: "/b".to_string(),
            base: Some(json!(5)),
            parent: None,
            child: Some(json!(5)),
            parent_kind: ParentKind::Delete,
        },
    )
    .expect("takes the template deletion");
    assert_eq!(root, json!({ "a": 2 }));
}

#[test]
fn set_pointer_refuses_instead_of_panicking() {
    // A scalar intermediate (the ancestor/descendant conflict shape: the
    // child wrote `5` where the template edits `x` inside), a missing array
    // position, a terminal index past the end, and a malformed pointer each
    // refuse and leave the tree untouched.
    let mut root = json!({ "system": { "obj": 5, "xs": [1] } });
    let before = root.clone();
    assert_eq!(
        set_pointer(&mut root, "/system/obj/x", json!(2)),
        Err(PointerError::NotAContainer),
        "the scalar is met at the terminal step"
    );
    assert_eq!(
        set_pointer(&mut root, "/system/obj/x/y", json!(2)),
        Err(PointerError::NotAContainer),
        "the scalar is met at an intermediate step"
    );
    assert_eq!(
        set_pointer(&mut root, "/system/xs/3/y", json!(2)),
        Err(PointerError::NotAContainer)
    );
    assert_eq!(
        set_pointer(&mut root, "/system/xs/3", json!(2)),
        Err(PointerError::IndexOutOfRange)
    );
    assert_eq!(
        set_pointer(&mut root, "", json!(2)),
        Err(PointerError::Malformed)
    );
    assert_eq!(
        set_pointer(&mut root, "system/obj", json!(2)),
        Err(PointerError::Malformed)
    );
    assert_eq!(delete_pointer(&mut root, ""), Err(PointerError::Malformed));
    assert_eq!(root, before);

    // A set-kind conflict without a parent value cannot be taken.
    assert_eq!(
        take_template(
            &mut root,
            &MergeConflict {
                path: "/system/obj".to_string(),
                base: None,
                parent: None,
                child: Some(json!(5)),
                parent_kind: ParentKind::Set,
            },
        ),
        Err(PointerError::MissingValue)
    );
}
