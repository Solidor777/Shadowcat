#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::data::document::Document;

/// The schema version current builds emit. No migrations exist pre-ship;
/// `migrate` is the machinery only and is a no-op at this version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Coerce a document to the current schema version. No migration steps are
/// registered pre-ship, so every document passes through unchanged; version
/// dispatch toward `CURRENT_SCHEMA_VERSION` is added with the first real step.
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::migrate::{migrate, CURRENT_SCHEMA_VERSION};
///
/// let doc = Document {
///     id: uuid::Uuid::nil(),
///     scope: Scope::Compendium { pack: "core".into() },
///     doc_type: "note".into(),
///     schema_version: CURRENT_SCHEMA_VERSION,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: None,
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let migrated = migrate(doc.clone());
/// assert_eq!(migrated, doc); // no registered steps yet: unchanged
/// ```
pub fn migrate(doc: Document) -> Document {
    if doc.schema_version >= CURRENT_SCHEMA_VERSION {
        return doc;
    }
    // No registered steps yet.
    doc
}

#[cfg(test)]
mod tests;
