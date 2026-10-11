//! Note-tree parent placement.
//!
//! It is the `note` half of `check_parent_placement` in the `sqlite` module, in a sibling `impl`
//! block the same way the `assets` submodule holds `check_asset_folder_parent`.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::*;
use crate::data::engine::NOTE_DOC_TYPE;

impl SqliteRepository {
    /// Enforces the note-tree placement invariant for a Created/Moved `doc`
    /// (a no-op for every other doc_type): `parent_id`, when set, names a
    /// `note` in the same scope. `batch` holds the documents this same
    /// command already Created — including a note Created earlier in this
    /// batch, so a same-command parent+child note pair resolves without a
    /// database round trip — consulted before the database, mirroring
    /// `check_asset_folder_parent`'s own batch-then-database resolution
    /// order exactly.
    pub(super) async fn check_note_parent(
        tx: &mut sqlx::SqliteConnection,
        doc: &Document,
        batch: &std::collections::HashMap<Uuid, Document>,
    ) -> Result<(), DataError> {
        if doc.doc_type != NOTE_DOC_TYPE {
            return Ok(());
        }
        let Some(pid) = doc.parent_id else {
            return Ok(());
        };
        let parent = match batch.get(&pid) {
            Some(d) => Some(d.clone()),
            None => Self::load_document(&mut *tx, pid).await?,
        };
        if !parent.is_some_and(|p| p.doc_type == NOTE_DOC_TYPE && p.scope == doc.scope) {
            return Err(DataError::OpFailed(
                "note parent must be a note in the same world".into(),
            ));
        }
        Ok(())
    }
}
