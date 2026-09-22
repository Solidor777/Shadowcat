use super::*;

#[test]
fn current_version_document_is_unchanged() {
    let doc = Document {
        id: crate::test_ids::test_uuid(1),
        scope: crate::data::document::Scope::World {
            world_id: crate::test_ids::test_uuid(9),
        },
        doc_type: "actor".into(),
        schema_version: CURRENT_SCHEMA_VERSION,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: Default::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: None,
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    };
    assert_eq!(migrate(doc.clone()), doc);
}
