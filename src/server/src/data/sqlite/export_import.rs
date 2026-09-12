//! World bundle export/import: the `WorldExportData` / `WorldImportData` half
//! of `SqliteRepository`, in a sibling `impl` block so `sqlite.rs` stays under
//! the file-size limit.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::*;

impl SqliteRepository {
    /// Every world-scoped row `delete_world` would delete, read instead — the
    /// per-world export data source. `users(id)` references are resolved to
    /// portable usernames inline (one `LEFT JOIN`/`JOIN` per table, no N+1
    /// lookups) exactly as documented on each `data::world_bundle::Exported*Row`
    /// type. `NotFound` if `world` does not exist.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let world = repo.create_world("MOCK_WORLD", 0).await?;
    /// let export = repo.export_world_rows(world.id).await?;
    /// assert_eq!(export.manifest.world_id, world.id);
    /// assert!(export.documents.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn export_world_rows(&self, world: Uuid) -> Result<WorldExportData, DataError> {
        let world_row =
            sqlx::query("SELECT name, seq, created_at, updated_at FROM worlds WHERE id = ?")
                .bind(world.to_string())
                .fetch_optional(&self.pool)
                .await?
                .ok_or(DataError::NotFound)?;

        let doc_rows = sqlx::query(
            "SELECT documents.json AS json, documents.seq AS seq, \
             documents.created_seq AS created_seq, users.username AS owner_username \
             FROM documents LEFT JOIN users ON users.id = documents.owner_id \
             WHERE documents.world_id = ? ORDER BY documents.id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut documents = Vec::with_capacity(doc_rows.len());
        for r in doc_rows {
            let mut document: Document = serde_json::from_str(&r.get::<String, _>("json"))?;
            document.owner = None;
            documents.push(ExportedDocumentRow {
                document,
                owner_username: r.get::<Option<String>, _>("owner_username"),
                seq: r.get("seq"),
                created_seq: r.get("created_seq"),
            });
        }

        let event_rows = sqlx::query(
            "SELECT world_events.seq AS seq, world_events.ts AS ts, \
             world_events.command_json AS command_json, users.username AS author_username \
             FROM world_events LEFT JOIN users ON users.id = world_events.author_id \
             WHERE world_events.world_id = ? ORDER BY world_events.seq",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let events: Vec<ExportedEventRow> = event_rows
            .into_iter()
            .map(|r| ExportedEventRow {
                seq: r.get("seq"),
                author_username: r.get::<Option<String>, _>("author_username"),
                ts: r.get("ts"),
                command_json: r.get("command_json"),
            })
            .collect();

        let member_rows = sqlx::query(
            "SELECT users.username AS username, world_members.role AS role \
             FROM world_members JOIN users ON users.id = world_members.user_id \
             WHERE world_members.world_id = ? ORDER BY users.username",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut members = Vec::with_capacity(member_rows.len());
        for r in member_rows {
            let role: WorldRole =
                serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?;
            members.push(ExportedMemberRow {
                username: r.get("username"),
                role,
            });
        }

        let invite_rows = sqlx::query(
            "SELECT world_invites.id AS id, world_invites.secret_hash AS secret_hash, \
             world_invites.role AS role, world_invites.created_at AS created_at, \
             world_invites.expires_at AS expires_at, world_invites.revoked_at AS revoked_at, \
             world_invites.consumed_at AS consumed_at, \
             creator.username AS created_by_username, consumer.username AS consumed_by_username \
             FROM world_invites \
             LEFT JOIN users creator ON creator.id = world_invites.created_by \
             LEFT JOIN users consumer ON consumer.id = world_invites.consumed_by \
             WHERE world_invites.world_id = ? ORDER BY world_invites.id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut invites = Vec::with_capacity(invite_rows.len());
        for r in invite_rows {
            let role: WorldRole =
                serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?;
            invites.push(ExportedInviteRow {
                id: Uuid::parse_str(r.get::<String, _>("id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?,
                secret_hash: r.get("secret_hash"),
                role,
                created_by_username: r.get::<Option<String>, _>("created_by_username"),
                created_at: r.get("created_at"),
                expires_at: r.get("expires_at"),
                revoked_at: r.get::<Option<i64>, _>("revoked_at"),
                consumed_at: r.get::<Option<i64>, _>("consumed_at"),
                consumed_by_username: r.get::<Option<String>, _>("consumed_by_username"),
            });
        }

        let asset_rows = sqlx::query(
            "SELECT assets.*, users.username AS created_by_username \
             FROM assets LEFT JOIN users ON users.id = assets.created_by \
             WHERE assets.world_id = ? ORDER BY assets.id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut full: Vec<crate::data::asset::Asset> = asset_rows
            .iter()
            .map(Self::asset_from_row)
            .collect::<Result<_, _>>()?;
        self.fill_tags(&mut full).await?;
        let mut assets = Vec::with_capacity(asset_rows.len());
        for (r, a) in asset_rows.iter().zip(full) {
            assets.push(ExportedAssetRow {
                id: a.id,
                original_name: a.original_name,
                content_type: a.content_type,
                byte_size: a.byte_size,
                created_by_username: r.get::<Option<String>, _>("created_by_username"),
                created_at: a.created_at,
                version: a.version,
                folder_id: a.folder_id,
                tags: a.tags,
                derived_tags: a.derived_tags,
                meta: a.meta,
            });
        }

        let fog_rows = sqlx::query(
            "SELECT explored_fog.scene_id AS scene_id, explored_fog.cells AS cells, \
             users.username AS username \
             FROM explored_fog JOIN users ON users.id = explored_fog.user_id \
             WHERE explored_fog.world_id = ? ORDER BY explored_fog.scene_id, users.username",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut fog = Vec::with_capacity(fog_rows.len());
        for r in fog_rows {
            fog.push(ExportedFogRow {
                scene_id: Uuid::parse_str(r.get::<String, _>("scene_id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?,
                username: r.get("username"),
                cells: r.get("cells"),
            });
        }

        let mut settings = Vec::new();
        for key in world_settings_keys(world) {
            let value: Option<String> =
                sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
                    .bind(&key)
                    .fetch_optional(&self.pool)
                    .await?;
            if let Some(value) = value {
                settings.push(ExportedSettingRow { key, value });
            }
        }

        let mut row_counts = std::collections::BTreeMap::new();
        row_counts.insert("documents".to_string(), documents.len());
        row_counts.insert("world_events".to_string(), events.len());
        row_counts.insert("world_members".to_string(), members.len());
        row_counts.insert("world_invites".to_string(), invites.len());
        row_counts.insert("assets".to_string(), assets.len());
        row_counts.insert("explored_fog".to_string(), fog.len());
        row_counts.insert("settings".to_string(), settings.len());

        let manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA_VERSION,
            world_id: world,
            world_name: world_row.get("name"),
            world_seq: world_row.get("seq"),
            world_created_at: world_row.get("created_at"),
            world_updated_at: world_row.get("updated_at"),
            exported_at_unix_ms: crate::ws::time::now_millis(),
            row_counts,
        };

        Ok(WorldExportData {
            manifest,
            documents,
            events,
            members,
            invites,
            assets,
            fog,
            settings,
        })
    }

    /// Resolve a portable username to a target-local user id inside `tx`, or
    /// `None` when `username` is `None` (no source owner) OR the username
    /// does not exist on this server — the degradation
    /// `documents.owner_id`/`world_events.author_id`/
    /// `world_invites.{created_by,consumed_by}` are already `ON DELETE SET
    /// NULL`-designed around.
    async fn resolve_username_tx(
        tx: &mut sqlx::SqliteConnection,
        username: Option<&str>,
    ) -> Result<Option<Uuid>, DataError> {
        let Some(username) = username else {
            return Ok(None);
        };
        let id: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&mut *tx)
            .await?;
        id.map(|s| Uuid::parse_str(&s).map_err(|e| DataError::OpFailed(e.to_string())))
            .transpose()
    }

    /// Insert one imported document row with EXPLICIT `seq`/`created_seq`,
    /// independently preserved from the source server — unlike the live
    /// write path's `upsert_document`, where a fresh Create always sets
    /// `seq == created_seq`. Shares `document_row_columns`/
    /// `reindex_document_fts` with `upsert_document` (search state is
    /// rebuilt from `doc`'s content, never carried across servers —
    /// `documents_fts_public`/`documents_fts_gm` are never exported/imported
    /// directly, see `data::world_bundle`'s module doc; the same is true of
    /// `assets_fts`, whose triggers rebuild it from the imported `assets`/
    /// `asset_tags` rows as `insert_asset`/`set_asset_tags` write them). A
    /// plain `INSERT`
    /// (not `upsert_document`'s `ON CONFLICT(id) DO UPDATE`): a document id
    /// colliding with an existing row anywhere on the target server (a
    /// separate axis from the already-gated world-id collision) is a
    /// genuine data-integrity fault, and letting the `UNIQUE` constraint
    /// violation surface as an ordinary `DataError::Sqlx` — aborting and
    /// rolling back the whole import transaction — is exactly the "any
    /// row-insert failure mid-transaction rolls back the whole import"
    /// behavior `import_world` already provides, not a case needing special
    /// handling. Callers must run `doc` through the same ingress validation
    /// every live write path runs (`import_world`'s own per-document loop
    /// does) before calling this — this function itself performs none.
    async fn insert_imported_document(
        conn: &mut sqlx::SqliteConnection,
        doc: &Document,
        seq: i64,
        created_seq: i64,
    ) -> Result<(), DataError> {
        let (scope_kind, world_id, pack, source_id, source_pack, source_version) =
            Self::document_row_columns(doc);
        let json = serde_json::to_string(doc)?;
        sqlx::query(
            "INSERT INTO documents (id, scope_kind, world_id, pack, doc_type, schema_version, \
             source_id, source_pack, source_version, owner_id, parent_id, seq, created_seq, json, \
             created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        .bind(created_seq)
        .bind(json)
        .bind(doc.created_at)
        .bind(doc.updated_at)
        .execute(&mut *conn)
        .await?;
        Self::reindex_document_fts(conn, doc, world_id).await
    }

    /// Import one `WorldImportData` bundle in a single transaction: reject a
    /// world-id collision with `worlds.id` before any row is written, insert
    /// `worlds` then every table in FK-safe order (`documents`/
    /// `world_events`/`world_members`/`world_invites`/`assets`, then the
    /// FK-less `explored_fog`/`settings`), resolving each row's portable
    /// username(s) against THIS server's `users` table, then finalize every
    /// staged asset file (rename into place beside itself — see
    /// `data::world_bundle::WorldImportData.staged_assets`) before
    /// committing — a failure at any point (including a rename) drops the
    /// transaction unrolled-back, so no partial world is ever visible.
    /// `world_members`/`explored_fog` rows whose username does not resolve
    /// are DROPPED (their `user_id` column is `NOT NULL`, so there is no
    /// `SET NULL` degradation to fall back to, unlike the four nullable
    /// owner/author/created_by/consumed_by columns) — counted in the
    /// returned `ImportSummary` rather than silently absorbed. Every
    /// document is run through the same ingress-validation chokepoint the
    /// live `Create`/`Update` write paths use
    /// (`validation::validate_system_size`/`validate_property_overrides`/
    /// `validate_engine_tree`/`validate_system_schema_tree`) before it
    /// reaches storage, and — once every row is inserted — through the same
    /// placement rules the `Operation::Create` arm of `apply_intent` runs
    /// (`validation::validate_containment`, `check_parent_placement`, and
    /// the self-parent rejection `Self::self_parent_error`), against the
    /// full imported set so a parent inserted later in the loop still
    /// resolves regardless of id order — an
    /// imported bundle is untrusted input to THIS server even when it was
    /// exported by a trusted admin from another one. Holds the pool's single
    /// writer connection for the entire call, including the asset-rename
    /// loop — every other server write (chat, moves, document edits) blocks
    /// for the whole import, the same trade-off `POST /api/admin/backup`
    /// already accepts for its snapshot.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// use shadowcat::data::world_bundle::{BundleManifest, WorldImportData, BUNDLE_SCHEMA_VERSION};
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let world_id = uuid::Uuid::new_v4();
    /// let data = WorldImportData {
    ///     manifest: BundleManifest {
    ///         schema_version: BUNDLE_SCHEMA_VERSION,
    ///         world_id,
    ///         world_name: "MOCK_WORLD".into(),
    ///         world_seq: 0,
    ///         world_created_at: 0,
    ///         world_updated_at: 0,
    ///         exported_at_unix_ms: 0,
    ///         row_counts: Default::default(),
    ///     },
    ///     documents: vec![],
    ///     events: vec![],
    ///     members: vec![],
    ///     invites: vec![],
    ///     assets: vec![],
    ///     fog: vec![],
    ///     settings: vec![],
    ///     staged_assets: vec![],
    ///     staged_siblings: vec![],
    /// };
    /// let summary = repo.import_world(data).await?;
    /// assert_eq!(summary.world_id, world_id);
    /// assert!(repo.get_world(world_id).await?.is_some());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn import_world(&self, data: WorldImportData) -> Result<ImportSummary, DataError> {
        let mut tx = self.pool.begin().await?;
        let world = data.manifest.world_id;

        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM worlds WHERE id = ?")
            .bind(world.to_string())
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_some() {
            return Err(DataError::Conflict(format!(
                "world {world} already exists on this server"
            )));
        }

        // Every `assets` row must have a matching staged file, or the
        // finalize loop below would leave a DB row with no backing bytes on
        // a truncated/malformed bundle. Checked up front, before any row is
        // written, so this failure mode is atomic like every other
        // `import_world` rejection.
        let staged_ids: std::collections::HashSet<Uuid> =
            data.staged_assets.iter().map(|(id, _)| *id).collect();
        for row in &data.assets {
            if !staged_ids.contains(&row.id) {
                return Err(DataError::OpFailed(format!(
                    "asset {} has no corresponding staged file in the bundle",
                    row.id
                )));
            }
        }

        sqlx::query(
            "INSERT INTO worlds (id, name, seq, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(world.to_string())
        .bind(&data.manifest.world_name)
        .bind(data.manifest.world_seq)
        .bind(data.manifest.world_created_at)
        .bind(data.manifest.world_updated_at)
        .execute(&mut *tx)
        .await?;

        // The tier-2 structural schema registry `validate_system_schema_tree`
        // needs, read from the BUNDLE's own imported `settings` rows rather
        // than `self.world_schema_declarations(world)` — that method queries
        // `self.pool` via a fresh connection, which would deadlock against
        // the transaction already held here under this server's
        // `max_connections(1)` single-writer pool. Mirrors
        // `world_schema_declarations`'s own `None => Vec::new()` default.
        let world_schemas: Vec<SchemaDeclaration> = data
            .settings
            .iter()
            .find(|s| s.key == world_schemas_key(world))
            .map(|s| serde_json::from_str(&s.value))
            .transpose()?
            .unwrap_or_default();

        // Mirrors `apply_intent`'s intra-batch `claimed_singletons` tracking
        // (see `SINGLETON_DOC_TYPES`'s own doc) — a bundle is untrusted
        // input assembled outside any live `apply_intent` call, so nothing
        // else in this loop would otherwise catch two documents of the same
        // singleton doc_type both landing in one import.
        let mut claimed_singletons: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // Every inserted document's normalized post-image, kept for the
        // post-loop placement pass below — `document`, not `row.document`,
        // since `validate_engine_tree` re-normalizes `engine` in place and
        // this is the form actually persisted.
        let mut inserted_documents: Vec<Document> = Vec::with_capacity(data.documents.len());
        // `export_world_rows` orders `documents` by id, which carries no
        // relationship to parent/child structure, and `documents.parent_id`
        // is an immediate (non-`DEFERRABLE`) foreign key — inserting a child
        // row before its not-yet-existing in-bundle parent fails that FK
        // regardless of whether the tree is valid. Reorder to a parent-
        // before-child insertion order wherever the bundle's OWN documents
        // make that possible (a parent outside the bundle, or absent, is
        // "ready" immediately; the FK still governs it), via Kahn's
        // algorithm (source: Kahn 1962): `indegree[i]` counts row `i`'s own
        // not-yet-placed in-bundle parent (0 or 1, since a document has at
        // most one parent), `queue` holds every currently-ready row index,
        // and placing a row decrements its recorded children's indegree via
        // `children`, at most once per row — O(n) rather than the repeated
        // full-remaining-set scan a naive parent-before-child reorder would
        // do (each earlier pass there frees only one document on a linear
        // chain). A document whose parent chain never resolves inside the
        // bundle is a cycle: none of its members ever reach indegree 0, so
        // none ever enters `queue`, and they are left in their original
        // `ORDER BY id` position at the tail — their `INSERT` still fails
        // the same immediate FK exactly as without this reordering, which is
        // what `import_world_rejects_a_two_note_mutual_cycle_via_the_immediate_fk`
        // pins.
        let doc_ids: std::collections::HashSet<Uuid> =
            data.documents.iter().map(|r| r.document.id).collect();
        // `documents.id TEXT PRIMARY KEY` is unique per server, so a bundle
        // naming the same document id twice can never be placed as two
        // distinct rows; the Kahn pass below also relies on each id
        // decrementing exactly one `children` entry per occurrence, so two
        // rows sharing an id would double-decrement a shared child's
        // indegree and underflow it. Rejected here, before that pass ever
        // runs, rather than let either failure mode surface.
        if doc_ids.len() != data.documents.len() {
            let mut seen: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
            let mut duplicate_ids: Vec<Uuid> = Vec::new();
            let mut reported: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
            for row in &data.documents {
                if !seen.insert(row.document.id) && reported.insert(row.document.id) {
                    duplicate_ids.push(row.document.id);
                }
            }
            let names = duplicate_ids
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(DataError::OpFailed(format!(
                "bundle contains duplicate document id(s): {names}"
            )));
        }
        let row_count = data.documents.len();
        let mut indegree: Vec<u8> = Vec::with_capacity(row_count);
        let mut children: std::collections::HashMap<Uuid, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, row) in data.documents.iter().enumerate() {
            match row.document.parent_id {
                Some(pid) if doc_ids.contains(&pid) => {
                    indegree.push(1);
                    children.entry(pid).or_default().push(i);
                }
                _ => indegree.push(0),
            }
        }
        let mut queue: std::collections::VecDeque<usize> =
            (0..row_count).filter(|&i| indegree[i] == 0).collect();
        let mut ordered_rows: Vec<&ExportedDocumentRow> = Vec::with_capacity(row_count);
        while let Some(i) = queue.pop_front() {
            ordered_rows.push(&data.documents[i]);
            if let Some(kids) = children.get(&data.documents[i].document.id) {
                for &child_idx in kids {
                    indegree[child_idx] -= 1;
                    if indegree[child_idx] == 0 {
                        queue.push_back(child_idx);
                    }
                }
            }
        }
        if ordered_rows.len() < row_count {
            ordered_rows.extend(
                data.documents
                    .iter()
                    .enumerate()
                    .filter(|&(i, _)| indegree[i] != 0)
                    .map(|(_, row)| row),
            );
        }

        for row in ordered_rows {
            let owner = Self::resolve_username_tx(&mut tx, row.owner_username.as_deref()).await?;
            let mut document = row.document.clone();
            document.owner = owner;
            if SINGLETON_DOC_TYPES.contains(&document.doc_type.as_str())
                && !claimed_singletons.insert(document.doc_type.clone())
            {
                return Err(DataError::Conflict(format!(
                    "bundle contains more than one '{}' document, which is capped at one per world",
                    document.doc_type
                )));
            }
            // A document's OWN scope must name the target world before
            // anything else about it is trusted — `document_row_columns`
            // (called by `insert_imported_document` below) persists
            // `world_id`/`scope_kind` straight from `document.scope`, and
            // `check_parent_placement`'s scope check below only ever runs
            // against a document's PARENT, never against the document
            // itself. Without this, a leaf/root document whose `scope`
            // names another existing world (or a compendium) would insert
            // with no scope check anywhere in this function.
            check_command_scope(&document, world)?;
            // Same ingress-validation chokepoint every live `Create`/`Update`
            // runs before a document reaches storage (see e.g. the
            // `Operation::Update` handler in `apply_intent`) — an imported
            // bundle is untrusted input, not a trusted internal write.
            // `validate_engine_tree` mutates `document.engine` in place
            // (re-normalizes it); the persisted row must hold that
            // normalized form, same as every other write path.
            validation::validate_system_size(&document)?;
            validation::validate_property_overrides(&document)?;
            validation::validate_engine_tree(&mut document)?;
            validation::validate_system_schema_tree(&document, &world_schemas)?;
            Self::insert_imported_document(&mut tx, &document, row.seq, row.created_seq).await?;
            inserted_documents.push(document);
        }

        // Placement checks the live `Operation::Create` arm of `apply_intent`
        // also runs, deferred to ONE pass after every row above is already
        // inserted: `validate_containment` (parent-shape rules — a `combat`/
        // `table` never parented, a `combatant`/`combat-history` always
        // parented), `check_parent_placement` (parent-TYPE rules — an
        // `asset_folder`/`note` parent must be same-type same-scope, a
        // `combatant`/`combat-history` parent must be a combat), and the
        // explicit self-parent rejection (`Self::self_parent_error`), in the
        // same order the Create arm applies them so a doubly-invalid
        // document reports the same error on both paths. Deferred rather
        // than run inside the loop above because `export_world_rows` orders
        // documents by id, so a per-row check would spuriously reject a
        // valid tree whose child sorts before its parent. Every imported
        // document is already committed to `tx` by this point, so
        // `check_parent_placement` is called exactly as the trusted
        // `apply_command` Move arm calls it: earlier ops in this command are
        // already applied, so the batch bookkeeping maps are empty by
        // construction — a stored parent resolves through `Self::
        // load_document` inside `check_parent_placement` itself, not
        // through a batch map. No general multi-hop cycle walk runs here:
        // `documents.parent_id`'s foreign key is enforced immediately (no
        // `DEFERRABLE`, `PRAGMA foreign_keys = ON` in `db.rs`), so a ≥2-node
        // `parent_id` cycle's first `INSERT` above already fails before this
        // pass is reached.
        for document in &inserted_documents {
            validation::validate_containment(document)?;
            Self::check_parent_placement(
                &mut tx,
                world,
                document,
                &Default::default(),
                &Default::default(),
            )
            .await?;
            if document.parent_id == Some(document.id) {
                return Err(Self::self_parent_error());
            }
        }

        for row in &data.events {
            let author = Self::resolve_username_tx(&mut tx, row.author_username.as_deref()).await?;
            sqlx::query(
                "INSERT INTO world_events (world_id, seq, author_id, ts, command_json) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(world.to_string())
            .bind(row.seq)
            .bind(author.map(|u| u.to_string()))
            .bind(row.ts)
            .bind(&row.command_json)
            .execute(&mut *tx)
            .await?;
        }

        let mut skipped_members = 0usize;
        for row in &data.members {
            match Self::resolve_username_tx(&mut tx, Some(row.username.as_str())).await? {
                Some(user_id) => {
                    sqlx::query(
                        "INSERT INTO world_members (world_id, user_id, role) VALUES (?, ?, ?)",
                    )
                    .bind(world.to_string())
                    .bind(user_id.to_string())
                    .bind(
                        serde_json::to_value(row.role)?
                            .as_str()
                            .expect("WorldRole serializes as a string")
                            .to_string(),
                    )
                    .execute(&mut *tx)
                    .await?;
                }
                None => skipped_members += 1,
            }
        }

        for row in &data.invites {
            let created_by =
                Self::resolve_username_tx(&mut tx, row.created_by_username.as_deref()).await?;
            let consumed_by =
                Self::resolve_username_tx(&mut tx, row.consumed_by_username.as_deref()).await?;
            sqlx::query(
                "INSERT INTO world_invites \
                 (id, world_id, secret_hash, role, created_by, created_at, expires_at, \
                  revoked_at, consumed_at, consumed_by) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row.id.to_string())
            .bind(world.to_string())
            .bind(&row.secret_hash)
            .bind(
                serde_json::to_value(row.role)?
                    .as_str()
                    .expect("WorldRole serializes as a string")
                    .to_string(),
            )
            .bind(created_by.map(|u| u.to_string()))
            .bind(row.created_at)
            .bind(row.expires_at)
            .bind(row.revoked_at)
            .bind(row.consumed_at)
            .bind(consumed_by.map(|u| u.to_string()))
            .execute(&mut *tx)
            .await?;
        }

        for row in &data.assets {
            let created_by =
                Self::resolve_username_tx(&mut tx, row.created_by_username.as_deref()).await?;
            let storage_key = format!("{world}/{}", row.id);
            // `original_retained` is only true if the bundle actually carried
            // the `.orig` sibling for this asset.
            let has_orig = data
                .staged_siblings
                .iter()
                .any(|s| s.asset_id == row.id && s.suffix == ".orig");
            let meta = crate::data::asset::AssetMeta {
                original_retained: row.meta.original_retained && has_orig,
                ..row.meta.clone()
            };
            sqlx::query(
                "INSERT INTO assets \
                 (id, world_id, storage_key, original_name, content_type, byte_size, created_by, \
                  created_at, version, folder_id, width, height, has_alpha, animated, \
                  original_content_type, original_byte_size, original_retained, conversion_note, \
                  duration_ms, sample_rate) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row.id.to_string())
            .bind(world.to_string())
            .bind(storage_key)
            .bind(&row.original_name)
            .bind(&row.content_type)
            .bind(row.byte_size)
            .bind(created_by.map(|u| u.to_string()))
            .bind(row.created_at)
            .bind(row.version)
            .bind(row.folder_id.map(|f| f.to_string()))
            .bind(meta.width.map(i64::from))
            .bind(meta.height.map(i64::from))
            .bind(i64::from(meta.has_alpha))
            .bind(i64::from(meta.animated))
            .bind(&meta.original_content_type)
            .bind(meta.original_byte_size)
            .bind(i64::from(meta.original_retained))
            .bind(&meta.conversion_note)
            .bind(meta.duration_ms)
            .bind(meta.sample_rate)
            .execute(&mut *tx)
            .await?;
            // A bundle is untrusted input: its explicit tags pass the same rule
            // every live writer applies (`tags::normalize_tags`); derived tags
            // are pipeline output and are re-derived on the next refresh.
            let explicit = crate::data::asset::tags::normalize_tags(row.tags.clone())
                .map_err(|m| DataError::OpFailed(format!("asset {} tags: {m}", row.id)))?;
            for (tag, derived) in explicit
                .iter()
                .map(|t| (t, 0_i64))
                .chain(row.derived_tags.iter().map(|t| (t, 1_i64)))
            {
                sqlx::query(
                    "INSERT OR IGNORE INTO asset_tags (asset_id, tag, derived) VALUES (?, ?, ?)",
                )
                .bind(row.id.to_string())
                .bind(tag)
                .bind(derived)
                .execute(&mut *tx)
                .await?;
            }
        }

        let mut skipped_fog = 0usize;
        for row in &data.fog {
            match Self::resolve_username_tx(&mut tx, Some(row.username.as_str())).await? {
                Some(user_id) => {
                    sqlx::query(
                        "INSERT INTO explored_fog (world_id, scene_id, user_id, cells) \
                         VALUES (?, ?, ?, ?)",
                    )
                    .bind(world.to_string())
                    .bind(row.scene_id.to_string())
                    .bind(user_id.to_string())
                    .bind(row.cells.as_slice())
                    .execute(&mut *tx)
                    .await?;
                }
                None => skipped_fog += 1,
            }
        }

        for row in &data.settings {
            sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?)")
                .bind(&row.key)
                .bind(&row.value)
                .execute(&mut *tx)
                .await?;
        }

        // Finalize staged asset files: rename each staged temp file (already
        // living in the target world's asset directory, per
        // `world_bundle::read_bundle`) to its final `<id>` name in that same
        // directory, only after every row above has been accepted by the
        // transaction. A failure here still rolls the WHOLE transaction back
        // (the early `?` return drops `tx` unrolled-back), and best-effort
        // removes every staged/finalized file so a rolled-back import leaves
        // no orphan bytes behind.
        // Canonicals and siblings finalize through one list: each staged
        // file renames to `<id><suffix>` ("" for the canonical) in place.
        let moves: Vec<(String, &std::path::PathBuf)> = data
            .staged_assets
            .iter()
            .map(|(id, staged)| (id.to_string(), staged))
            .chain(
                data.staged_siblings
                    .iter()
                    .map(|s| (format!("{}{}", s.asset_id, s.suffix), &s.staged)),
            )
            .collect();
        let mut finalized: Vec<std::path::PathBuf> = Vec::with_capacity(moves.len());
        for (name, staged) in &moves {
            let dest = staged
                .parent()
                .expect("staged asset path always has a parent directory")
                .join(name);
            if let Err(e) = tokio::fs::rename(staged, &dest).await {
                for done in &finalized {
                    let _ = tokio::fs::remove_file(done).await;
                }
                for (_, remaining) in &moves {
                    let _ = tokio::fs::remove_file(remaining).await;
                }
                return Err(DataError::OpFailed(format!(
                    "failed to finalize imported asset file {name}: {e}"
                )));
            }
            finalized.push(dest);
        }

        // Accepted low-likelihood risk: if every asset above renamed
        // successfully but this commit itself then fails, the renamed files
        // remain on disk with no `assets` row (and no `worlds` row at all)
        // referencing them — an orphan, not a visible/reachable partial
        // world. A SQLite commit failure this late (all statements already
        // succeeded) is rare; no compensating cleanup is implemented for it.
        tx.commit().await?;

        Ok(ImportSummary {
            world_id: world,
            skipped_members,
            skipped_fog,
        })
    }
}
