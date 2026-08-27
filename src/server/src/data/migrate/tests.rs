use super::*;
use uuid::Uuid;

#[test]
fn current_version_document_is_unchanged() {
    let doc = Document {
        id: Uuid::from_u128(1),
        scope: crate::data::document::Scope::World {
            world_id: Uuid::from_u128(9),
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
