//! This module holds the table-folder half of `check_parent_placement` and the folder-delete
//! reparent step.
//!
//! It is a sibling `impl` block, the same way the `notes` submodule holds `check_note_parent`.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::*;
use crate::data::engine::{TABLE_DOC_TYPE, TABLE_FOLDER_DOC_TYPE};

impl SqliteRepository {
    /// Enforces the table-folder placement invariant for a Created/Moved `doc`
    /// (a no-op for every doc_type but `table` and `table_folder`):
    /// `parent_id`, when set, names a `table_folder` in the same scope.
    /// `batch` holds the documents this same command already Created —
    /// including a folder Created earlier in this batch, so a same-command
    /// folder+table pair resolves without a database round trip — consulted
    /// before the database, mirroring `check_note_parent`'s own
    /// batch-then-database resolution order exactly.
    pub(super) async fn check_table_folder_parent(
        tx: &mut sqlx::SqliteConnection,
        doc: &Document,
        batch: &std::collections::HashMap<Uuid, Document>,
    ) -> Result<(), DataError> {
        if doc.doc_type != TABLE_DOC_TYPE && doc.doc_type != TABLE_FOLDER_DOC_TYPE {
            return Ok(());
        }
        let Some(pid) = doc.parent_id else {
            return Ok(());
        };
        let parent = match batch.get(&pid) {
            Some(d) => Some(d.clone()),
            None => Self::load_document(&mut *tx, pid).await?,
        };
        if !parent.is_some_and(|p| p.doc_type == TABLE_FOLDER_DOC_TYPE && p.scope == doc.scope) {
            return Err(DataError::OpFailed(format!(
                "{} parent must be a table_folder in the same world",
                doc.doc_type
            )));
        }
        Ok(())
    }

    /// The ids a list of `Move` ops (from `table_folder_reparent_moves`) relocate, for the
    /// `kept` argument of `descendants_first`.
    pub(super) fn moved_ids(moves: &[Operation]) -> std::collections::HashSet<Uuid> {
        moves
            .iter()
            .filter_map(|op| match op {
                Operation::Move { doc_id, .. } => Some(*doc_id),
                _ => None,
            })
            .collect()
    }

    /// The ids of every `Delete` op in `ops`: the set of documents the command removes, which
    /// `table_folder_reparent_moves` must not pick as a lift target.
    pub(super) fn deleted_ids(ops: &[Operation]) -> std::collections::HashSet<Uuid> {
        ops.iter()
            .filter_map(|op| match op {
                Operation::Delete { doc } => Some(doc.id),
                _ => None,
            })
            .collect()
    }

    /// The `Move` ops that lift every `table` out of the `table_folder` `folder_id`'s subtree
    /// before that folder is deleted: each table filed anywhere under the folder or one of its
    /// nested sub-folders is moved to the nearest ancestor of the folder that the command does
    /// not also delete (`doomed` holds every id the command deletes; `None` = world root).
    /// A command that deletes an outer folder and a nested one therefore lifts the tables to
    /// the same surviving ancestor in either order, never onto a folder about to be deleted.
    /// The nested sub-folders themselves are left for the caller's descendant expansion, which
    /// deletes them. Empty for a missing document and for every doc_type but `table_folder`.
    ///
    /// The lift is part of the folder's delete, not a separate Move: it is authorized by the
    /// delete gate the caller already applied to the folder, so it does not pass the GM-only gate
    /// an explicit `Move` op meets. A folder owner who may delete the folder therefore also
    /// re-files the tables beneath it, which keeps them reachable instead of orphaning or
    /// deleting them.
    ///
    /// Both authoritative write loops (`apply_command`, `apply_intent`) call this at the
    /// start of a `Delete`'s expansion, so a folder delete never removes a table by either
    /// path. The moves are explicit ops, so they are logged, broadcast and invertible; a
    /// silent column update would leave every client's store with the old parent.
    pub(super) async fn table_folder_reparent_moves(
        tx: &mut sqlx::SqliteConnection,
        folder_id: Uuid,
        doomed: &std::collections::HashSet<Uuid>,
    ) -> Result<Vec<Operation>, DataError> {
        let Some(folder) = Self::load_document(&mut *tx, folder_id).await? else {
            return Ok(Vec::new());
        };
        if folder.doc_type != TABLE_FOLDER_DOC_TYPE {
            return Ok(Vec::new());
        }
        // Walk up past every ancestor this command also deletes; the visited set
        // bounds the walk on a cyclic parent chain.
        let mut target = folder.parent_id;
        let mut climbed = std::collections::HashSet::from([folder_id]);
        while let Some(p) = target {
            if !doomed.contains(&p) || !climbed.insert(p) {
                break;
            }
            target = Self::load_document(&mut *tx, p)
                .await?
                .and_then(|d| d.parent_id);
        }
        // Breadth-first closure of `table_folder` descendants; the visited set
        // bounds the walk on a self-referencing or cyclic parent chain.
        let mut closure = vec![folder_id];
        let mut seen = std::collections::HashSet::from([folder_id]);
        let mut next = 0;
        while next < closure.len() {
            let rows = sqlx::query(
                "SELECT id FROM documents WHERE parent_id = ? AND doc_type = ? ORDER BY id",
            )
            .bind(closure[next].to_string())
            .bind(TABLE_FOLDER_DOC_TYPE)
            .fetch_all(&mut *tx)
            .await?;
            next += 1;
            for r in rows {
                let child = Uuid::parse_str(r.get::<String, _>("id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                if seen.insert(child) {
                    closure.push(child);
                }
            }
        }
        let mut moves = Vec::new();
        for holder in closure {
            let rows = sqlx::query(
                "SELECT id FROM documents WHERE parent_id = ? AND doc_type = ? ORDER BY id",
            )
            .bind(holder.to_string())
            .bind(TABLE_DOC_TYPE)
            .fetch_all(&mut *tx)
            .await?;
            for r in rows {
                let table = Uuid::parse_str(r.get::<String, _>("id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                moves.push(Operation::Move {
                    doc_id: table,
                    parent_id: target,
                    old_parent_id: Some(holder),
                });
            }
        }
        Ok(moves)
    }
}
