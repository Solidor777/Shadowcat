//! Unit tests for the embedded-collection machinery not directly visible in
//! the corpus's normalized ids (fresh-id minting, provenance version
//! preservation, `base` clearing).

use serde_json::json;

use crate::merge::embedded::restamp_subtree;

use super::{doc, test_id};

#[test]
fn restamp_subtree_mints_fresh_ids_and_repoints_source_recursively() {
    let mut grandchild = doc("gc1");
    grandchild.system = json!({ "deep": 1 });
    let mut child = doc("tc1");
    child.embedded.insert("sub".to_string(), vec![grandchild]);
    let mut template = doc("t1");
    template.embedded.insert("items".to_string(), vec![child]);

    let stamped = restamp_subtree(&template);
    assert_ne!(stamped.id, test_id("t1"));
    assert_eq!(stamped.source.as_ref().map(|s| s.id), Some(test_id("t1")));
    let stamped_child = &stamped.embedded["items"][0];
    assert_ne!(stamped_child.id, test_id("tc1"));
    assert_eq!(
        stamped_child.source.as_ref().map(|s| s.id),
        Some(test_id("tc1"))
    );
}

#[test]
fn restamp_subtree_preserves_the_source_version_and_clears_base() {
    let mut template = doc("t1");
    template.source = Some(crate::data::document::Source {
        id: test_id("origin"),
        pack: None,
        version: 3,
    });
    template.base = Some(json!({ "name": null }));

    let stamped = restamp_subtree(&template);
    assert_eq!(stamped.source.as_ref().map(|s| s.version), Some(3));
    // The pack is NOT inherited: the template's own provenance pack is
    // unrelated to this stamp.
    assert_eq!(stamped.source.as_ref().and_then(|s| s.pack.as_ref()), None);
    assert_eq!(stamped.base, None);
}
