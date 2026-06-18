use crate::data::document::Document;

/// The schema version current builds emit. No migrations exist pre-ship;
/// `migrate` is the machinery only and is a no-op at this version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Coerce a document to the current schema version. No migration steps are
/// registered pre-ship, so every document passes through unchanged; version
/// dispatch toward `CURRENT_SCHEMA_VERSION` is added with the first real step.
pub fn migrate(doc: Document) -> Document {
    if doc.schema_version >= CURRENT_SCHEMA_VERSION {
        return doc;
    }
    // No registered steps yet.
    doc
}

#[cfg(test)]
mod tests {
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
            source: None,
            owner: None,
            permissions: Default::default(),
            embedded: Default::default(),
            system: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        };
        assert_eq!(migrate(doc.clone()), doc);
    }
}
