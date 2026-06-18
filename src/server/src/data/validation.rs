use crate::data::document::Document;
use crate::data::DataError;

/// Maximum serialized size of a document's opaque `system` body.
pub const MAX_SYSTEM_BYTES: usize = 256 * 1024;

/// Reject documents whose opaque body exceeds the size cap.
pub fn validate_system_size(doc: &Document) -> Result<(), DataError> {
    let bytes = serde_json::to_vec(&doc.system)?.len();
    if bytes > MAX_SYSTEM_BYTES {
        return Err(DataError::TooLarge(bytes));
    }
    Ok(())
}

/// A valid JSON pointer is empty or a sequence of "/"-prefixed tokens.
pub fn validate_field_path(path: &str) -> Result<(), DataError> {
    if path.is_empty() {
        return Ok(());
    }
    if !path.starts_with('/') {
        return Err(DataError::BadPath(path.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn doc_with_system(system: serde_json::Value) -> Document {
        Document {
            id: Uuid::from_u128(1),
            scope: crate::data::document::Scope::World {
                world_id: Uuid::from_u128(9),
            },
            doc_type: "actor".into(),
            schema_version: 1,
            source: None,
            owner: None,
            permissions: Default::default(),
            embedded: Default::default(),
            system,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn small_system_passes() {
        assert!(validate_system_size(&doc_with_system(serde_json::json!({ "hp": 1 }))).is_ok());
    }

    #[test]
    fn oversized_system_is_rejected() {
        let big = "x".repeat(MAX_SYSTEM_BYTES + 1);
        let err = validate_system_size(&doc_with_system(serde_json::json!({ "blob": big })));
        assert!(matches!(err, Err(DataError::TooLarge(_))));
    }

    #[test]
    fn field_paths_validate() {
        assert!(validate_field_path("").is_ok());
        assert!(validate_field_path("/system/hp").is_ok());
        assert!(matches!(
            validate_field_path("system/hp"),
            Err(DataError::BadPath(_))
        ));
    }
}
