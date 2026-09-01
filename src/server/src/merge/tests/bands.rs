//! Unit tests for the band types, snapshot builder, and placement
//! exclusions.

use serde_json::json;

use crate::merge::bands::{is_placement_excluded, placement_exclusions, snapshot_base};

use super::{doc, source_from};

#[test]
fn placement_exclusions_only_cover_token_placement() {
    assert_eq!(
        placement_exclusions("token"),
        ["/engine/x", "/engine/y", "/engine/rotation"]
    );
    assert!(placement_exclusions("actor").is_empty());
}

#[test]
fn is_placement_excluded_matches_exact_and_descendant_paths_only() {
    let excl = placement_exclusions("token");
    assert!(is_placement_excluded("/engine/x", &excl));
    assert!(is_placement_excluded("/engine/x/deep", &excl));
    assert!(!is_placement_excluded("/engine/xylophone", &excl));
    assert!(!is_placement_excluded("/engine/hp", &excl));
}

#[test]
fn snapshot_base_keys_children_by_source_id_and_coalesces_absent_bands() {
    let mut kid = doc("ic1");
    kid.name = Some("Kid".to_string());
    kid.source = Some(source_from("tc1"));
    kid.system = json!({ "hp": 3 });
    let mut d = doc("c1");
    d.name = Some("Inst".to_string());
    d.engine = Some(json!({ "hp": 9 }));
    d.system = json!({ "a": 1 });
    d.embedded.insert("items".to_string(), vec![kid]);

    let snap = snapshot_base(&d);
    assert_eq!(snap.name.as_deref(), Some("Inst"));
    assert_eq!(snap.engine, json!({ "hp": 9 }));
    assert_eq!(snap.system, json!({ "a": 1 }));
    let kids = &snap.embedded["items"];
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].source_id, super::test_id("tc1").to_string());
    assert_eq!(kids[0].name.as_deref(), Some("Kid"));
    assert_eq!(kids[0].engine, json!(null));
    assert_eq!(kids[0].system, json!({ "hp": 3 }));
}

#[test]
fn snapshot_base_falls_back_to_own_id_for_non_provenance_children() {
    let kid = doc("local1");
    let mut d = doc("c1");
    d.embedded.insert("items".to_string(), vec![kid]);
    let snap = snapshot_base(&d);
    assert_eq!(
        snap.embedded["items"][0].source_id,
        super::test_id("local1").to_string()
    );
}

#[test]
fn merge_base_deserialization_defaults_missing_bands() {
    // A partial historical record reads with `null`/empty bands, matching the
    // client engine's `?? null` coalescing.
    let parsed: crate::merge::bands::MergeBase =
        serde_json::from_value(json!({ "system": { "hp": 1 } })).expect("partial base parses");
    assert_eq!(parsed.name, None);
    assert_eq!(parsed.engine, json!(null));
    assert_eq!(parsed.system, json!({ "hp": 1 }));
    assert!(parsed.embedded.is_empty());
}
