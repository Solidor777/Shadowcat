//! Document rows: the `documents` / `documents_fts_gm` / `documents_fts_public`
//! half of `SqliteRepository` — row load/upsert/delete with FTS reindex,
//! parent placement and move-cycle checks, and the commit-time `OpSnapshot`
//! builder — in a sibling `impl` block so `sqlite.rs` stays under the
//! file-size limit.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::*;

impl SqliteRepository {
    /// The error a self-referential `parent_id` raises: a document naming
    /// itself as its own parent satisfies the self-FK and commits, then
    /// poisons deletion's descendant walk (it would loop). Shared by
    /// `apply_intent`'s Create arm and `import_world`'s post-loop placement
    /// pass so the two checks never state the message twice.
    pub(super) fn self_parent_error() -> DataError {
        DataError::OpFailed("document cannot be its own parent".into())
    }

    /// Load a document envelope by id on an arbitrary executor (so it can run
    /// inside a transaction). Mirrors `get_document`'s row→Document mapping.
    pub(super) async fn load_document<'e, E>(
        executor: E,
        id: Uuid,
    ) -> Result<Option<Document>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row = sqlx::query("SELECT json FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(executor)
            .await?;
        match row {
            Some(r) => Ok(Some(serde_json::from_str(
                r.get::<String, _>("json").as_str(),
            )?)),
            None => Ok(None),
        }
    }

    /// Resolve `doc`'s effective owner (`permission::effective_owner`) on an
    /// arbitrary executor, joining the LINKED actor for a token so the rule is
    /// evaluated against LIVE actor state on every write — nothing is stamped,
    /// so re-assigning an actor's owner immediately re-owns its linked tokens.
    /// Runs on the caller's transaction (never `&self.pool`, which would
    /// deadlock mid-transaction on the single-writer pool).
    ///
    /// Costs ONE extra row read, and only for a token carrying an `actor_id`
    /// link. This function performs the JOIN and nothing else: precedence
    /// between the override and the inherited owner is decided EXCLUSIVELY by
    /// `effective_owner`. Deliberately does NOT skip the read when `doc.owner`
    /// is set — re-deriving "the override wins" here would duplicate the rule in
    /// a second place that can silently drift from it.
    pub(super) async fn load_effective_owner<'e, E>(
        executor: E,
        doc: &Document,
    ) -> Result<Option<Uuid>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let Some(actor_id) = crate::data::permission::token_actor_link(doc) else {
            return Ok(crate::data::permission::effective_owner(doc, None));
        };
        // A dangling link loads `None` and `effective_owner` fails closed to no owner.
        //
        // `load_document` is keyed on id alone (no `world_id` filter), so a cross-world
        // `actor_id` would otherwise resolve. The cross-world scope check lives inside
        // `permission::effective_owner` itself — see that function's doc comment for the
        // rationale (keeps the reachable set equal to `SceneEcs.actors` by construction).
        let actor = Self::load_document(executor, actor_id).await?;
        Ok(crate::data::permission::effective_owner(
            doc,
            actor.as_ref(),
        ))
    }

    /// `documents.created_seq` for `id`, or `None` if the row doesn't exist. Set once at a
    /// row's genuine first INSERT (`upsert_document`'s `ON CONFLICT` clause omits it, so
    /// SQLite's `excluded.*` semantics leave it untouched across an update) and never touched
    /// again by subsequent updates to a still-live row — the generation marker
    /// `OpSnapshot::created_seq_at_commit` compares against to detect an id reused after a hard
    /// delete. Runs on the caller's transaction (never `&self.pool`, which would deadlock
    /// mid-transaction on the single-writer pool).
    pub(super) async fn document_created_seq<'e, E>(
        executor: E,
        id: Uuid,
    ) -> Result<Option<i64>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row = sqlx::query("SELECT created_seq FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(executor)
            .await?;
        Ok(row.map(|r| r.get::<i64, _>("created_seq")))
    }

    /// Every CURRENT member's world role, on an arbitrary executor (so it can run inside the
    /// `apply_command`/`apply_intent` transaction). Feeds `CommandSnapshot::world_gm_at_commit`
    /// — captured once per command, at the point the command is committing, which IS "at commit
    /// time" for this purpose: the whole point of capturing it now is to freeze what would
    /// otherwise be re-derived live on every future replay.
    pub(super) async fn world_member_roles<'e, E>(
        executor: E,
        world_id: Uuid,
    ) -> Result<std::collections::HashMap<Uuid, WorldRole>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let rows = sqlx::query("SELECT user_id, role FROM world_members WHERE world_id = ?")
            .bind(world_id.to_string())
            .fetch_all(executor)
            .await?;
        rows.into_iter()
            .map(|r| {
                let uid = Uuid::parse_str(r.get::<String, _>("user_id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                let role: WorldRole =
                    serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?;
                Ok((uid, role))
            })
            .collect()
    }

    /// Build one op's commit-time redaction snapshot from the command's FINAL post-image state
    /// (`post_images`, accumulated across the WHOLE mutation loop) and, for a `Delete`, its
    /// created_seq captured BEFORE the row was removed (`deleted_created_seqs` — the row is gone
    /// by the time this runs, so it cannot be read here). Runs on the caller's open transaction,
    /// after every op in the command has applied and every write has landed. Shared by
    /// `apply_command` and `apply_intent` — the ONE place either loop computes a snapshot, so
    /// they cannot diverge.
    pub(super) async fn build_op_snapshot(
        tx: &mut sqlx::SqliteConnection,
        op: &Operation,
        post_images: &std::collections::HashMap<Uuid, Document>,
        deleted_created_seqs: &std::collections::HashMap<Uuid, i64>,
        pre_permissions: &std::collections::HashMap<Uuid, crate::data::document::PermissionSet>,
        pre_owners: &std::collections::HashMap<Uuid, Option<Uuid>>,
    ) -> Result<crate::data::snapshot::OpSnapshot, DataError> {
        use crate::data::snapshot::OpSnapshot;
        match op {
            Operation::Create { doc } => {
                // Reads the command's FINAL post-image, never `doc` (this op's own
                // per-iteration intermediate state): a same-command op that later mutates
                // this same id (e.g. an Update reassigning `/owner` or adding an override)
                // must be reflected in the Create's own persisted snapshot, or a stale value
                // is stored forever in `world_events.command_json`. Guaranteed `Some` — both
                // `apply_command` and `apply_intent` unconditionally insert into
                // `post_images` immediately after a Create's own document write.
                let doc = post_images.get(&doc.id).ok_or_else(|| {
                    DataError::OpFailed(format!(
                        "post-image missing for created document {}",
                        doc.id
                    ))
                })?;
                let owner_at_commit = Self::load_effective_owner(&mut *tx, doc).await?;
                let mut overrides_at_commit = Vec::new();
                crate::data::permission::collect_overrides(doc, "", &mut overrides_at_commit)
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                Ok(OpSnapshot {
                    owner_at_commit,
                    doc_type: doc.doc_type.clone(),
                    overrides_at_commit,
                    retraction_hidden_at_commit: None,
                    created_seq_at_commit: None,
                    permissions_at_commit: None,
                    permissions_before_commit: None,
                    owner_before_commit: None,
                })
            }
            // Reads the op's OWN carried `doc`, not `post_images` — unlike Create/Update,
            // there is no coherent "final state" for an id deleted within this same
            // command: `post_images` holds no entry for it (the mutation loop never
            // inserts one on Delete), and a later op resurrecting the same id via a fresh
            // Create is a distinct document, not a continuation of this one.
            Operation::Delete { doc } => {
                let owner_at_commit = Self::load_effective_owner(&mut *tx, doc).await?;
                let mut overrides_at_commit = Vec::new();
                crate::data::permission::collect_overrides(doc, "", &mut overrides_at_commit)
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                Ok(OpSnapshot {
                    owner_at_commit,
                    doc_type: doc.doc_type.clone(),
                    overrides_at_commit,
                    retraction_hidden_at_commit: None,
                    created_seq_at_commit: deleted_created_seqs.get(&doc.id).copied(),
                    permissions_at_commit: None,
                    permissions_before_commit: None,
                    owner_before_commit: None,
                })
            }
            Operation::Update { doc_id, changes } => {
                let doc = post_images.get(doc_id).ok_or_else(|| {
                    DataError::OpFailed(format!("post-image missing for updated document {doc_id}"))
                })?;
                let owner_at_commit = Self::load_effective_owner(&mut *tx, doc).await?;
                let mut overrides_full = Vec::new();
                crate::data::permission::collect_overrides(doc, "", &mut overrides_full)
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                let touches_perms = changes
                    .iter()
                    .any(|c| crate::data::permission::touches_permissions(&c.path));
                let retraction_hidden_at_commit = if touches_perms {
                    Some(overrides_full.clone())
                } else {
                    None
                };
                // Pruned to the ancestor/descendant closure of this op's own changed paths —
                // only an overlapping override can possibly redact THIS op's field-level deltas.
                let overrides_at_commit: Vec<(String, crate::data::document::Visibility)> =
                    overrides_full
                        .into_iter()
                        .filter(|(p, _)| {
                            changes
                                .iter()
                                .any(|c| crate::data::permission::paths_overlap(p, &c.path))
                        })
                        .collect();
                let created_seq_at_commit = Self::document_created_seq(&mut *tx, *doc_id).await?;
                Ok(OpSnapshot {
                    owner_at_commit,
                    doc_type: doc.doc_type.clone(),
                    overrides_at_commit,
                    retraction_hidden_at_commit,
                    created_seq_at_commit,
                    permissions_at_commit: Some(crate::data::document::PermissionSet {
                        property_overrides: Default::default(),
                        ..doc.permissions.clone()
                    }),
                    permissions_before_commit: pre_permissions.get(doc_id).map(|p| {
                        crate::data::document::PermissionSet {
                            property_overrides: Default::default(),
                            ..p.clone()
                        }
                    }),
                    owner_before_commit: pre_owners.get(doc_id).copied().flatten(),
                })
            }
            // Mirrors the Update arm's post-image sourcing without its
            // change-delta pieces: a Move carries no `FieldChange`s, so the
            // path-overlap pruning yields an empty override set and no
            // retraction capture (a Move never touches `permissions`).
            Operation::Move { doc_id, .. } => {
                let doc = post_images.get(doc_id).ok_or_else(|| {
                    DataError::OpFailed(format!("post-image missing for moved document {doc_id}"))
                })?;
                let owner_at_commit = Self::load_effective_owner(&mut *tx, doc).await?;
                let created_seq_at_commit = Self::document_created_seq(&mut *tx, *doc_id).await?;
                Ok(OpSnapshot {
                    owner_at_commit,
                    doc_type: doc.doc_type.clone(),
                    overrides_at_commit: Vec::new(),
                    retraction_hidden_at_commit: None,
                    created_seq_at_commit,
                    permissions_at_commit: Some(crate::data::document::PermissionSet {
                        property_overrides: Default::default(),
                        ..doc.permissions.clone()
                    }),
                    permissions_before_commit: pre_permissions.get(doc_id).map(|p| {
                        crate::data::document::PermissionSet {
                            property_overrides: Default::default(),
                            ..p.clone()
                        }
                    }),
                    owner_before_commit: pre_owners.get(doc_id).copied().flatten(),
                })
            }
        }
    }

    /// Parent-placement checks the Create AND Move arms share — the one
    /// statement of "may a document of this type sit under this parent",
    /// covering the checks that need the database or the batch bookkeeping:
    /// a stored parent must belong to this command's world
    /// (`check_command_scope`), a `combatant`/`combat-history` parent must be
    /// a combat (batch-aware), an `asset_folder` parent must be a
    /// same-scope folder (`check_asset_folder_parent`, batch-aware), and a
    /// `note` parent must be a same-scope note (`check_note_parent`,
    /// batch-aware). A parent this same batch Creates is not in the database
    /// yet — it resolves through the batch maps, and its own Create was
    /// scope-checked; a parent that exists nowhere yet is left to the
    /// self-FK at apply time, so batched parent+child creates still pass.
    /// `validate_containment` (pure placement shape) runs separately at
    /// every caller.
    pub(super) async fn check_parent_placement(
        tx: &mut sqlx::SqliteConnection,
        world_id: Uuid,
        doc: &Document,
        batch_folders: &std::collections::HashMap<Uuid, Document>,
        batch_combats: &std::collections::HashSet<Uuid>,
    ) -> Result<(), DataError> {
        if doc.doc_type == COMBATANT_DOC_TYPE || doc.doc_type == COMBAT_HISTORY_DOC_TYPE {
            // `validate_containment` already guarantees `parent_id` is
            // `Some` for a combatant/combat-history document.
            let pid = doc.parent_id.expect(
                "validate_containment requires a combatant/combat-history doc to carry a parent_id",
            );
            let stored_parent = if batch_combats.contains(&pid) {
                None
            } else {
                Self::load_document(&mut *tx, pid).await?
            };
            if let Some(parent) = &stored_parent {
                check_command_scope(parent, world_id)?;
            }
            let parent_is_combat = batch_combats.contains(&pid)
                || stored_parent.is_some_and(|p| p.doc_type == COMBAT_DOC_TYPE);
            if !parent_is_combat {
                return Err(DataError::OpFailed(format!(
                    "{} parent must be a combat document",
                    doc.doc_type
                )));
            }
        } else if let Some(pid) = doc.parent_id {
            // Every other doc_type: no parent-TYPE rule, but a stored parent
            // still belongs to this command's world.
            if !batch_folders.contains_key(&pid) && !batch_combats.contains(&pid) {
                if let Some(parent) = Self::load_document(&mut *tx, pid).await? {
                    check_command_scope(&parent, world_id)?;
                }
            }
        }
        Self::check_asset_folder_parent(&mut *tx, doc, batch_folders).await?;
        Self::check_note_parent(&mut *tx, doc, batch_folders).await?;
        Ok(())
    }

    /// Rejects a Move that would parent `moved` beneath itself: walks the
    /// ancestor chain upward from `new_parent`, resolving each hop against
    /// this batch's not-yet-applied Moves first (`batch_moves` — the
    /// prospective parent wins over the stored one, since the walk must see
    /// the tree the batch will leave, and Phase 2 applies nothing until
    /// every op has validated), then this batch's not-yet-inserted Creates
    /// (`batch_folders`), then the stored tree — refusing if `moved` appears
    /// anywhere in the chain (self-parent included).
    /// Bounded: a chain deeper than `MAX_MOVE_ANCESTRY` (or a stored cycle,
    /// which cannot arise but would otherwise loop) is refused, not walked.
    pub(super) async fn check_move_acyclic(
        tx: &mut sqlx::SqliteConnection,
        moved: Uuid,
        new_parent: Option<Uuid>,
        batch_folders: &std::collections::HashMap<Uuid, Document>,
        batch_moves: &std::collections::HashMap<Uuid, Option<Uuid>>,
    ) -> Result<(), DataError> {
        /// Depth bound for the ancestor walk; no legitimate tree approaches it.
        const MAX_MOVE_ANCESTRY: u32 = 1_000;
        let mut cursor = new_parent;
        let mut hops = 0u32;
        while let Some(pid) = cursor {
            if pid == moved {
                return Err(DataError::OpFailed(
                    "a document cannot be moved beneath itself".into(),
                ));
            }
            hops += 1;
            if hops > MAX_MOVE_ANCESTRY {
                return Err(DataError::OpFailed(
                    "parent chain too deep to verify".into(),
                ));
            }
            cursor = if let Some(prospective) = batch_moves.get(&pid) {
                *prospective
            } else {
                match batch_folders.get(&pid) {
                    Some(batch_doc) => batch_doc.parent_id,
                    None => Self::load_document(&mut *tx, pid)
                        .await?
                        .and_then(|d| d.parent_id),
                }
            };
        }
        Ok(())
    }

    /// Whether a document of `doc_type` already exists in `world_id`, on an
    /// arbitrary executor (so it can run inside the `apply_intent`
    /// transaction — see `SINGLETON_DOC_TYPES`). Mirrors `load_document`'s
    /// tx-generic pattern rather than `query_documents`, which always binds
    /// to `&self.pool` and would deadlock if called mid-transaction against
    /// this single-writer (`max_connections(1)`) pool.
    pub(super) async fn singleton_doc_exists<'e, E>(
        executor: E,
        world_id: Uuid,
        doc_type: &str,
    ) -> Result<bool, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row =
            sqlx::query("SELECT 1 FROM documents WHERE world_id = ? AND doc_type = ? LIMIT 1")
                .bind(world_id.to_string())
                .bind(doc_type)
                .fetch_optional(executor)
                .await?;
        Ok(row.is_some())
    }

    /// The id of the `combat` document currently `active: true` on `scene_id`
    /// in `world_id`, or `None` if the scene has no active combat. Runs on
    /// the caller's transaction for the same single-writer reason as
    /// `singleton_doc_exists`. At most one row can ever match, since this is
    /// the same one-active-combat-per-scene invariant `apply_intent`'s
    /// `scene_owner` map enforces.
    async fn active_combat_owner<'e, E>(
        executor: E,
        world_id: Uuid,
        scene_id: Uuid,
    ) -> Result<Option<Uuid>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row = sqlx::query(
            "SELECT id FROM documents WHERE world_id = ? AND doc_type = ? \
             AND json_extract(json, '$.engine.active') = 1 \
             AND json_extract(json, '$.engine.scene_id') = ? LIMIT 1",
        )
        .bind(world_id.to_string())
        .bind(COMBAT_DOC_TYPE)
        .bind(scene_id.to_string())
        .fetch_optional(executor)
        .await?;
        row.map(|r| {
            Uuid::parse_str(r.get::<String, _>("id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))
        })
        .transpose()
    }

    /// Lazily seeds `scene_owner[scene]` with the DB's current active-combat
    /// owner the first time this batch's simulation touches `scene`'s
    /// active-combat state (a no-op on every later touch, tracked by
    /// `seeded_scenes`). When `deactivations_this_batch` already names
    /// `scene` -- a genuine same-batch active-true-to-false transition found
    /// by `apply_intent`'s pre-scan -- `scene` is deliberately left
    /// unseeded (absent from `scene_owner`) rather than seeded from the
    /// stale DB row: the batch is itself about to free this scene, so an
    /// EARLIER same-batch claim on it must be validated against the state
    /// the batch will actually leave, not a DB read a LATER op in the same
    /// batch is about to invalidate.
    pub(super) async fn ensure_scene_owner_seeded<'e, E>(
        executor: E,
        world_id: Uuid,
        scene: Uuid,
        scene_owner: &mut std::collections::HashMap<Uuid, Uuid>,
        seeded_scenes: &mut std::collections::HashSet<Uuid>,
        deactivations_this_batch: &std::collections::HashSet<Uuid>,
    ) -> Result<(), DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        if !seeded_scenes.insert(scene) {
            return Ok(());
        }
        if deactivations_this_batch.contains(&scene) {
            return Ok(());
        }
        if let Some(owner) = Self::active_combat_owner(executor, world_id, scene).await? {
            scene_owner.insert(scene, owner);
        }
        Ok(())
    }

    /// Depth-first descendant ids of `root` within one transaction (children
    /// before parents), via the `parent_id` index. Excludes `root`. Used to
    /// expand a parent delete into per-descendant reversible Delete ops.
    ///
    /// The `seen` set (seeded with `root`) makes the walk terminate on any
    /// self-reference or cycle: a single `INSERT` whose `parent_id` equals its
    /// own `id` satisfies the self-FK and commits, so without this guard a
    /// `WHERE parent_id = root` row referencing `root` would recurse forever.
    pub(super) async fn descendants_first(
        tx: &mut sqlx::SqliteConnection,
        root: Uuid,
    ) -> Result<Vec<Uuid>, DataError> {
        let mut seen = std::collections::HashSet::from([root]);
        let mut out = Vec::new();
        Self::collect_descendants(tx, root, &mut seen, &mut out).await?;
        Ok(out)
    }

    /// Walk `parent_id` links breadth-first from `node`, collecting every
    /// descendant id into `seen` (visited-set bounds a cyclic self-FK walk).
    ///
    /// # Examples
    ///
    /// ```text
    /// Self::collect_descendants(&mut tx, scene_id, &mut seen).await?; // seen: subtree ids
    /// ```
    async fn collect_descendants(
        tx: &mut sqlx::SqliteConnection,
        node: Uuid,
        seen: &mut std::collections::HashSet<Uuid>,
        out: &mut Vec<Uuid>,
    ) -> Result<(), DataError> {
        let child_rows = sqlx::query("SELECT id FROM documents WHERE parent_id = ? ORDER BY id")
            .bind(node.to_string())
            .fetch_all(&mut *tx)
            .await?;
        for r in child_rows {
            let child = Uuid::parse_str(r.get::<String, _>("id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))?;
            // Skip already-visited nodes (self-reference / cycle guard).
            if !seen.insert(child) {
                continue;
            }
            // Recurse first so deeper descendants precede their parent.
            Box::pin(Self::collect_descendants(&mut *tx, child, seen, out)).await?;
            out.push(child);
        }
        Ok(())
    }

    /// Derives the `documents` row's scope/source columns from `doc`'s
    /// envelope — the exact derivation `upsert_document` and
    /// `insert_imported_document` both need before their (differing) INSERT
    /// statements, factored out once so the two document-write paths cannot
    /// silently diverge on it. Returns `(scope_kind, world_id, pack,
    /// source_id, source_pack, source_version)`.
    pub(super) fn document_row_columns(doc: &Document) -> DocumentRowColumns {
        let (scope_kind, world_id, pack) = match &doc.scope {
            Scope::Compendium { pack } => ("compendium", None, Some(pack.clone())),
            Scope::World { world_id } => ("world", Some(world_id.to_string()), None),
        };
        let (source_id, source_pack, source_version) = match &doc.source {
            Some(s) => (
                Some(s.id.to_string()),
                s.pack.clone(),
                Some(s.version as i64),
            ),
            None => (None, None, None),
        };
        (
            scope_kind,
            world_id,
            pack,
            source_id,
            source_pack,
            source_version,
        )
    }

    /// Rewrite `doc`'s FTS index rows (both visibility-tier tables) in the
    /// caller's transaction — the delete-then-reinsert block
    /// `upsert_document` and `insert_imported_document` both need after
    /// their (differing) `documents` INSERT, factored out once for the same
    /// never-fork reason as `document_row_columns`. `world_id` is passed in
    /// (rather than re-derived) because both callers already computed it via
    /// `document_row_columns`.
    ///
    /// Two SEPARATE single-column tables, not two columns of one table:
    /// bm25()'s row-length normalization is computed from the WHOLE ROW (all
    /// columns), so a shared table would let a non-GM query's score be
    /// shifted by the mere LENGTH of GM-only text on the same row even when
    /// column weights zero out that column's term-frequency contribution.
    /// Separate tables make each tier's row length genuinely isolated.
    pub(super) async fn reindex_document_fts(
        conn: &mut sqlx::SqliteConnection,
        doc: &Document,
        world_id: Option<String>,
    ) -> Result<(), DataError> {
        sqlx::query("DELETE FROM documents_fts_public WHERE doc_id = ?")
            .bind(doc.id.to_string())
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM documents_fts_gm WHERE doc_id = ?")
            .bind(doc.id.to_string())
            .execute(&mut *conn)
            .await?;
        sqlx::query(
            "INSERT INTO documents_fts_public (content, doc_id, world_id, doc_type) VALUES (?, ?, ?, ?)",
        )
        .bind(crate::data::search::index_content_public(doc))
        .bind(doc.id.to_string())
        .bind(world_id.clone())
        .bind(&doc.doc_type)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO documents_fts_gm (content_all, doc_id, world_id, doc_type) VALUES (?, ?, ?, ?)",
        )
        .bind(crate::data::search::index_content(doc))
        .bind(doc.id.to_string())
        .bind(world_id)
        .bind(&doc.doc_type)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Upsert a document row from its envelope, stamping `seq`, and rewrite its
    /// FTS index row in the same transaction (crash-consistent). Takes a
    /// `&mut SqliteConnection` because it runs multiple statements; callers pass
    /// `&mut *tx`.
    pub(super) async fn upsert_document(
        conn: &mut sqlx::SqliteConnection,
        doc: &Document,
        seq: i64,
    ) -> Result<(), DataError> {
        let (scope_kind, world_id, pack, source_id, source_pack, source_version) =
            Self::document_row_columns(doc);
        let json = serde_json::to_string(doc)?;
        sqlx::query(
            "INSERT INTO documents (id, scope_kind, world_id, pack, doc_type, schema_version, \
             source_id, source_pack, source_version, owner_id, parent_id, seq, created_seq, json, \
             created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET scope_kind=excluded.scope_kind, world_id=excluded.world_id, \
             pack=excluded.pack, doc_type=excluded.doc_type, schema_version=excluded.schema_version, \
             source_id=excluded.source_id, source_pack=excluded.source_pack, \
             source_version=excluded.source_version, owner_id=excluded.owner_id, \
             parent_id=excluded.parent_id, seq=excluded.seq, \
             json=excluded.json, updated_at=excluded.updated_at",
        )
        .bind(doc.id.to_string())
        .bind(scope_kind)
        .bind(world_id.clone())
        .bind(pack)
        .bind(&doc.doc_type)
        .bind(doc.schema_version as i64)
        .bind(source_id)
        .bind(source_pack)
        .bind(source_version)
        .bind(doc.owner.map(|o| o.to_string()))
        .bind(doc.parent_id.map(|p| p.to_string()))
        .bind(seq)
        .bind(seq)
        .bind(json)
        .bind(doc.created_at)
        .bind(doc.updated_at)
        .execute(&mut *conn)
        .await?;
        Self::reindex_document_fts(conn, doc, world_id).await
    }

    /// Remove a document's FTS rows (both visibility-tier tables). Call
    /// alongside every document delete so the index never references a
    /// removed document.
    async fn delete_document_fts(
        conn: &mut sqlx::SqliteConnection,
        id: Uuid,
    ) -> Result<(), DataError> {
        sqlx::query("DELETE FROM documents_fts_public WHERE doc_id = ?")
            .bind(id.to_string())
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM documents_fts_gm WHERE doc_id = ?")
            .bind(id.to_string())
            .execute(conn)
            .await?;
        Ok(())
    }

    /// Apply a document Delete inside `tx`: the row, its FTS entries, and its
    /// explored-fog rows. SINGLE SOURCE for delete side-effects — BOTH
    /// authoritative delete paths (`apply_intent`, `apply_command`) call this,
    /// so they cannot drift (never-fork). The fog purge is unconditional by id:
    /// only scene documents ever appear as `explored_fog.scene_id`, so it is a
    /// no-op for every other doc_type and carries no doc_type predicate that
    /// could drift from the fog writer's keying.
    pub(super) async fn delete_document_tx(
        tx: &mut sqlx::SqliteConnection,
        id: Uuid,
    ) -> Result<(), DataError> {
        // Asset-folder hook: every asset filed under a deleted folder moves
        // to the folder's parent BEFORE the row goes, so the `assets.folder_id`
        // FK's `SET NULL` never fires (that would flatten to root instead of
        // the parent). Parent deletes expand children-first, so a sub-folder's
        // assets hop one level per op and end in the surviving ancestor.
        Self::reparent_assets_of_deleted_folder(&mut *tx, id).await?;
        sqlx::query("DELETE FROM documents WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        Self::delete_document_fts(&mut *tx, id).await?;
        sqlx::query("DELETE FROM explored_fog WHERE scene_id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(())
    }

    /// Test-only raw insert that bypasses every ingress gate, including
    /// `apply_command`/`apply_intent`'s `/engine` normalization — seeds a
    /// `Document` exactly as given, malformed `engine` body included, to
    /// exercise a reader's fail-closed handling of already-persisted data
    /// that predates or violates the current typed schema (schema
    /// evolution, hand-edited rows). `apply_command` validates on write, so
    /// it cannot seed such fixtures; this is not a production code path and
    /// must stay `#[cfg(test)]`-only.
    #[cfg(test)]
    pub(crate) async fn seed_document_unvalidated(&self, doc: &Document) -> Result<(), DataError> {
        let mut tx = self.pool.begin().await?;
        Self::upsert_document(&mut tx, doc, 0).await?;
        tx.commit().await?;
        Ok(())
    }
}
