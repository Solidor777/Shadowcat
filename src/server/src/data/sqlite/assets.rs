//! Asset rows + tags: the `assets` / `asset_tags` half of `SqliteRepository`,
//! in a sibling `impl` block so `sqlite.rs` stays under the file-size limit.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::*;
use crate::data::asset::query::{AssetCursor, AssetFilter, AssetKind, AssetSort, FolderFilter};
use crate::data::asset::tags::{derive, provenance_of, DeriveInput};
use crate::data::asset::{Asset, AssetMeta};
use crate::data::engine::ASSET_FOLDER_DOC_TYPE;

/// Deepest folder nesting `folder_ancestor_names` walks before giving up on
/// a chain (defensive bound; the write gate keeps the tree acyclic).
const MAX_FOLDER_DEPTH: usize = 64;

/// One `asset_tags` row, in memory.
struct TagRow {
    /// The tag text.
    tag: String,
    /// `true` for a pipeline-derived tag, `false` for a GM-set one.
    derived: bool,
}

/// Split `rows` (already ordered by tag) into `(explicit, derived)` lists.
fn split_tags(rows: impl IntoIterator<Item = TagRow>) -> (Vec<String>, Vec<String>) {
    let mut explicit = Vec::new();
    let mut derived = Vec::new();
    for r in rows {
        if r.derived {
            derived.push(r.tag);
        } else {
            explicit.push(r.tag);
        }
    }
    (explicit, derived)
}

impl SqliteRepository {
    /// Enforces the `asset_folder` placement invariant for a Created `doc`
    /// (a no-op for every other doc_type): `parent_id`, when set, names an
    /// `asset_folder` in the same scope. `batch` holds the documents this
    /// same command already Created, consulted before the database so an
    /// in-batch parent resolves. No cycle walk is needed: `parent_id` is an
    /// immutable envelope path (`required_cap_for_path` maps it to `None`),
    /// so a folder's parent is fixed at Create, and a Create can only name a
    /// parent that already exists — stored (acyclic by induction) or earlier
    /// in this batch (strictly ordered) — which keeps the tree acyclic by
    /// construction.
    pub(super) async fn check_asset_folder_parent(
        tx: &mut sqlx::SqliteConnection,
        doc: &Document,
        batch: &std::collections::HashMap<Uuid, Document>,
    ) -> Result<(), DataError> {
        if doc.doc_type != ASSET_FOLDER_DOC_TYPE {
            return Ok(());
        }
        let Some(pid) = doc.parent_id else {
            return Ok(());
        };
        let parent = match batch.get(&pid) {
            Some(d) => Some(d.clone()),
            None => Self::load_document(&mut *tx, pid).await?,
        };
        if !parent.is_some_and(|p| p.doc_type == ASSET_FOLDER_DOC_TYPE && p.scope == doc.scope) {
            return Err(DataError::OpFailed(
                "asset_folder parent must be an asset_folder in the same world".into(),
            ));
        }
        Ok(())
    }

    /// If `id` is an `asset_folder`, moves every asset filed under it to the
    /// folder's own parent (`NULL` = root). Runs inside the document-delete
    /// transaction, before the row is removed.
    pub(super) async fn reparent_assets_of_deleted_folder(
        tx: &mut sqlx::SqliteConnection,
        id: Uuid,
    ) -> Result<(), DataError> {
        let row = sqlx::query("SELECT doc_type, parent_id FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
        let Some(row) = row else {
            return Ok(());
        };
        if row.get::<String, _>("doc_type") != ASSET_FOLDER_DOC_TYPE {
            return Ok(());
        }
        let parent: Option<String> = row.get("parent_id");
        let moved = sqlx::query("UPDATE assets SET folder_id = ? WHERE folder_id = ? RETURNING id")
            .bind(parent)
            .bind(id.to_string())
            .fetch_all(&mut *tx)
            .await?;
        for r in moved {
            let asset_id = Uuid::parse_str(r.get::<String, _>("id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))?;
            // The folder-segment tags name ancestors that just changed.
            Self::refresh_derived_tags_tx(&mut *tx, asset_id).await?;
        }
        Ok(())
    }

    /// `folder_ancestor_names` on a fresh connection.
    pub async fn folder_ancestor_names_of(
        &self,
        folder_id: Option<Uuid>,
    ) -> Result<Vec<String>, DataError> {
        let mut conn = self.pool.acquire().await?;
        Self::folder_ancestor_names(&mut conn, folder_id).await
    }

    /// Root-first names of `folder_id` and its ancestors (empty for `None`,
    /// the world root). Feeds the folder-segment derived tags. A nameless
    /// folder contributes nothing; the walk stops at `MAX_FOLDER_DEPTH`.
    pub(crate) async fn folder_ancestor_names(
        tx: &mut sqlx::SqliteConnection,
        folder_id: Option<Uuid>,
    ) -> Result<Vec<String>, DataError> {
        let mut names = Vec::new();
        let mut cur = folder_id;
        let mut hops = 0;
        while let Some(id) = cur {
            let Some(doc) = Self::load_document(&mut *tx, id).await? else {
                break;
            };
            if let Some(name) = doc.name {
                names.push(name);
            }
            hops += 1;
            if hops > MAX_FOLDER_DEPTH {
                break;
            }
            cur = doc.parent_id;
        }
        names.reverse();
        Ok(names)
    }

    /// Ids of every asset filed in `folder` or any folder beneath it.
    pub(crate) async fn assets_in_folder_subtree(
        tx: &mut sqlx::SqliteConnection,
        folder: Uuid,
    ) -> Result<Vec<Uuid>, DataError> {
        let rows = sqlx::query(
            "WITH RECURSIVE sub(id) AS (SELECT ? UNION ALL SELECT d.id FROM documents d \
             JOIN sub ON d.parent_id = sub.id WHERE d.doc_type = 'asset_folder') \
             SELECT a.id AS id FROM assets a WHERE a.folder_id IN (SELECT id FROM sub) \
             ORDER BY a.created_at, a.id",
        )
        .bind(folder.to_string())
        .fetch_all(&mut *tx)
        .await?;
        rows.iter()
            .map(|r| {
                Uuid::parse_str(r.get::<String, _>("id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))
            })
            .collect()
    }

    /// `assets_in_folder_subtree` on a fresh connection.
    pub async fn assets_in_folder_subtree_of(&self, folder: Uuid) -> Result<Vec<Uuid>, DataError> {
        let mut conn = self.pool.acquire().await?;
        Self::assets_in_folder_subtree(&mut conn, folder).await
    }

    /// Recompute the derived tags of every asset under `folder` (the folder
    /// itself included) inside `tx` — the folder-segment tags name every
    /// ancestor, so a rename anywhere above an asset changes its set.
    pub(crate) async fn refresh_derived_tags_for_folder_subtree(
        tx: &mut sqlx::SqliteConnection,
        folder: Uuid,
    ) -> Result<(), DataError> {
        for id in Self::assets_in_folder_subtree(&mut *tx, folder).await? {
            Self::refresh_derived_tags_tx(&mut *tx, id).await?;
        }
        Ok(())
    }

    /// GM placement edit of one asset, in one transaction: `name` and
    /// `folder` (`Some(None)` = move to root) when given, the explicit tag
    /// set replaced by `tags` when given, then the derived set refreshed.
    /// `None` when the asset does not exist.
    pub async fn update_asset_placement(
        &self,
        id: Uuid,
        name: Option<&str>,
        folder: Option<Option<Uuid>>,
        tags: Option<&[String]>,
    ) -> Result<Option<Asset>, DataError> {
        let mut tx = self.pool.begin().await?;
        let exists = sqlx::query("SELECT 1 FROM assets WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .is_some();
        if !exists {
            return Ok(None);
        }
        if let Some(name) = name {
            sqlx::query("UPDATE assets SET original_name = ? WHERE id = ?")
                .bind(name)
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        if let Some(folder) = folder {
            sqlx::query("UPDATE assets SET folder_id = ? WHERE id = ?")
                .bind(folder.map(|f| f.to_string()))
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        if let Some(tags) = tags {
            sqlx::query("DELETE FROM asset_tags WHERE asset_id = ? AND derived = 0")
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;
            for tag in tags {
                Self::set_explicit_tag(&mut tx, id, tag).await?;
            }
        }
        Self::refresh_derived_tags_tx(&mut tx, id).await?;
        tx.commit().await?;
        self.get_asset(id).await
    }

    /// Record `tag` as explicit on `id`; a derived row of the same text is
    /// promoted (the GM's intent outranks the derivation).
    async fn set_explicit_tag(
        tx: &mut sqlx::SqliteConnection,
        id: Uuid,
        tag: &str,
    ) -> Result<(), DataError> {
        sqlx::query(
            "INSERT INTO asset_tags (asset_id, tag, derived) VALUES (?, ?, 0) \
             ON CONFLICT(asset_id, tag) DO UPDATE SET derived = 0",
        )
        .bind(id.to_string())
        .bind(tag)
        .execute(&mut *tx)
        .await?;
        Ok(())
    }

    /// GM bulk placement edit, one transaction: every id must belong to
    /// `world` (else `NotFound`, nothing applied); `folder` (`Some(None)` =
    /// root) moves them all; `add_tags` are recorded as explicit;
    /// `remove_tags` drops explicit tags only (a derived tag cannot be
    /// removed — it would come straight back on the next refresh). Returns
    /// the updated assets in `ids` order.
    pub async fn bulk_update_assets(
        &self,
        world: Uuid,
        ids: &[Uuid],
        folder: Option<Option<Uuid>>,
        add_tags: &[String],
        remove_tags: &[String],
    ) -> Result<Vec<Asset>, DataError> {
        let mut tx = self.pool.begin().await?;
        for id in ids {
            let owner: Option<String> = sqlx::query("SELECT world_id FROM assets WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?
                .map(|r| r.get("world_id"));
            if owner.as_deref() != Some(world.to_string().as_str()) {
                return Err(DataError::NotFound);
            }
        }
        for id in ids {
            if let Some(folder) = folder {
                sqlx::query("UPDATE assets SET folder_id = ? WHERE id = ?")
                    .bind(folder.map(|f| f.to_string()))
                    .bind(id.to_string())
                    .execute(&mut *tx)
                    .await?;
            }
            for tag in add_tags {
                Self::set_explicit_tag(&mut tx, *id, tag).await?;
            }
            for tag in remove_tags {
                sqlx::query(
                    "DELETE FROM asset_tags WHERE asset_id = ? AND tag = ? AND derived = 0",
                )
                .bind(id.to_string())
                .bind(tag)
                .execute(&mut *tx)
                .await?;
            }
            Self::refresh_derived_tags_tx(&mut tx, *id).await?;
        }
        tx.commit().await?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(a) = self.get_asset(*id).await? {
                out.push(a);
            }
        }
        Ok(out)
    }

    /// `refresh_derived_tags_tx` in its own transaction.
    pub async fn refresh_derived_tags(&self, id: Uuid) -> Result<(), DataError> {
        let mut tx = self.pool.begin().await?;
        Self::refresh_derived_tags_tx(&mut tx, id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Recomputes and rewrites ONLY the derived (`derived = 1`) tag rows of
    /// asset `id` inside `tx`, from the stored row, its folder chain and the
    /// provenance its current derived set encodes. Explicit tags are untouched.
    pub(crate) async fn refresh_derived_tags_tx(
        tx: &mut sqlx::SqliteConnection,
        id: Uuid,
    ) -> Result<(), DataError> {
        let Some(row) = sqlx::query("SELECT * FROM assets WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?
        else {
            return Ok(());
        };
        let asset = Self::asset_from_row(&row)?;
        let old_derived: Vec<String> =
            sqlx::query("SELECT tag FROM asset_tags WHERE asset_id = ? AND derived = 1")
                .bind(id.to_string())
                .fetch_all(&mut *tx)
                .await?
                .iter()
                .map(|r| r.get::<String, _>("tag"))
                .collect();
        let folder_names = Self::folder_ancestor_names(&mut *tx, asset.folder_id).await?;
        let derived = derive(DeriveInput {
            content_type: &asset.content_type,
            meta: &asset.meta,
            folder_names: &folder_names,
            provenance: provenance_of(&old_derived),
        });
        sqlx::query("DELETE FROM asset_tags WHERE asset_id = ? AND derived = 1")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        for tag in derived {
            // An explicit tag of the same text already occupies the primary
            // key; the GM's copy wins, so the derived duplicate is skipped.
            sqlx::query(
                "INSERT OR IGNORE INTO asset_tags (asset_id, tag, derived) VALUES (?, ?, 1)",
            )
            .bind(id.to_string())
            .bind(tag)
            .execute(&mut *tx)
            .await?;
        }
        Ok(())
    }

    /// Insert a new asset record. `version` starts at 1. Tags are NOT written
    /// here — `set_asset_tags` owns both tag sets.
    pub async fn insert_asset(&self, a: &Asset) -> Result<(), DataError> {
        sqlx::query(
            "INSERT INTO assets \
             (id, world_id, storage_key, original_name, content_type, byte_size, created_by, \
              created_at, version, folder_id, width, height, has_alpha, animated, \
              original_content_type, original_byte_size, original_retained, conversion_note) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(a.id.to_string())
        .bind(a.world_id.to_string())
        .bind(&a.storage_key)
        .bind(&a.original_name)
        .bind(&a.content_type)
        .bind(a.byte_size)
        .bind(a.created_by.map(|u| u.to_string()))
        .bind(a.created_at)
        .bind(a.version)
        .bind(a.folder_id.map(|u| u.to_string()))
        .bind(a.meta.width.map(i64::from))
        .bind(a.meta.height.map(i64::from))
        .bind(i64::from(a.meta.has_alpha))
        .bind(i64::from(a.meta.animated))
        .bind(&a.meta.original_content_type)
        .bind(a.meta.original_byte_size)
        .bind(i64::from(a.meta.original_retained))
        .bind(&a.meta.conversion_note)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Map an `assets` row to the `Asset` struct (uuid columns parse from
    /// TEXT). `tags`/`derived_tags` come back EMPTY — the row carries no tag
    /// columns; callers fill them from `asset_tags`.
    ///
    /// # Examples
    ///
    /// ```text
    /// let asset = Self::asset_from_row(&row)?;
    /// ```
    pub(super) fn asset_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Asset, DataError> {
        let parse = |s: String| Uuid::parse_str(&s).map_err(|e| DataError::OpFailed(e.to_string()));
        let dim = |v: Option<i64>| v.and_then(|n| u32::try_from(n).ok());
        Ok(Asset {
            id: parse(row.get::<String, _>("id"))?,
            world_id: parse(row.get::<String, _>("world_id"))?,
            storage_key: row.get("storage_key"),
            original_name: row.get("original_name"),
            content_type: row.get("content_type"),
            byte_size: row.get("byte_size"),
            created_by: row
                .get::<Option<String>, _>("created_by")
                .map(parse)
                .transpose()?,
            created_at: row.get("created_at"),
            version: row.get("version"),
            folder_id: row
                .get::<Option<String>, _>("folder_id")
                .map(parse)
                .transpose()?,
            tags: Vec::new(),
            derived_tags: Vec::new(),
            meta: AssetMeta {
                width: dim(row.get::<Option<i64>, _>("width")),
                height: dim(row.get::<Option<i64>, _>("height")),
                has_alpha: row.get::<i64, _>("has_alpha") != 0,
                animated: row.get::<i64, _>("animated") != 0,
                original_content_type: row.get("original_content_type"),
                original_byte_size: row.get("original_byte_size"),
                original_retained: row.get::<i64, _>("original_retained") != 0,
                conversion_note: row.get("conversion_note"),
            },
        })
    }

    /// Both tag lists for one asset, each ordered by tag.
    async fn load_tags(&self, id: Uuid) -> Result<(Vec<String>, Vec<String>), DataError> {
        let rows =
            sqlx::query("SELECT tag, derived FROM asset_tags WHERE asset_id = ? ORDER BY tag")
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await?;
        Ok(split_tags(rows.iter().map(|r| TagRow {
            tag: r.get("tag"),
            derived: r.get::<i64, _>("derived") != 0,
        })))
    }

    /// Fetch one asset row by id (tags filled), or `None` if absent.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(repo.get_asset(uuid::Uuid::nil()).await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_asset(&self, id: Uuid) -> Result<Option<Asset>, DataError> {
        let row = sqlx::query("SELECT * FROM assets WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let mut asset = Self::asset_from_row(&row)?;
        let (tags, derived) = self.load_tags(id).await?;
        asset.tags = tags;
        asset.derived_tags = derived;
        Ok(Some(asset))
    }

    /// Swap the bytes behind a stable id: rewrites the served-file columns AND
    /// every pipeline-metadata column from `meta`; bumps and returns the new
    /// version.
    pub async fn replace_asset_bytes(
        &self,
        id: Uuid,
        storage_key: &str,
        content_type: &str,
        byte_size: i64,
        meta: &AssetMeta,
    ) -> Result<i64, DataError> {
        let v: i64 = sqlx::query(
            "UPDATE assets SET storage_key = ?, content_type = ?, byte_size = ?, \
             width = ?, height = ?, has_alpha = ?, animated = ?, original_content_type = ?, \
             original_byte_size = ?, original_retained = ?, conversion_note = ?, \
             version = version + 1 \
             WHERE id = ? RETURNING version",
        )
        .bind(storage_key)
        .bind(content_type)
        .bind(byte_size)
        .bind(meta.width.map(i64::from))
        .bind(meta.height.map(i64::from))
        .bind(i64::from(meta.has_alpha))
        .bind(i64::from(meta.animated))
        .bind(&meta.original_content_type)
        .bind(meta.original_byte_size)
        .bind(i64::from(meta.original_retained))
        .bind(&meta.conversion_note)
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DataError::NotFound)?
        .get("version");
        Ok(v)
    }

    /// Replace BOTH tag sets of `id` in one transaction: `explicit` becomes
    /// the GM-set list, `derived` the pipeline list. A tag present in both is
    /// stored once, as explicit (the GM's intent outranks the derivation).
    pub async fn set_asset_tags(
        &self,
        id: Uuid,
        explicit: &[String],
        derived: &[String],
    ) -> Result<(), DataError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM asset_tags WHERE asset_id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        let mut seen = std::collections::HashSet::new();
        for (tag, is_derived) in explicit
            .iter()
            .map(|t| (t, 0_i64))
            .chain(derived.iter().map(|t| (t, 1_i64)))
        {
            if !seen.insert(tag.as_str()) {
                continue;
            }
            sqlx::query("INSERT INTO asset_tags (asset_id, tag, derived) VALUES (?, ?, ?)")
                .bind(id.to_string())
                .bind(tag)
                .bind(is_derived)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Remove the record, returning it (so the caller can delete the file).
    /// Single atomic `DELETE ... RETURNING` so two concurrent deletes can't both
    /// observe the row and double-fire side effects (file remove + broadcast) —
    /// only the call that actually removes the row gets `Some`. Tag rows go
    /// with the `asset_tags` FK cascade; the returned struct carries none.
    pub async fn delete_asset(&self, id: Uuid) -> Result<Option<Asset>, DataError> {
        let row = sqlx::query("DELETE FROM assets WHERE id = ? RETURNING *")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| Self::asset_from_row(&r)).transpose()
    }

    /// Fill `tags`/`derived_tags` on every asset in `assets` with one query
    /// per 500 ids.
    async fn fill_tags(&self, assets: &mut [Asset]) -> Result<(), DataError> {
        for chunk in assets.chunks_mut(500) {
            let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
                "SELECT asset_id, tag, derived FROM asset_tags WHERE asset_id IN (",
            );
            let mut sep = qb.separated(", ");
            for a in chunk.iter() {
                sep.push_bind(a.id.to_string());
            }
            qb.push(") ORDER BY tag");
            let rows = qb.build().fetch_all(&self.pool).await?;
            let mut by_asset: std::collections::HashMap<Uuid, Vec<TagRow>> =
                std::collections::HashMap::new();
            for r in rows {
                let asset_id = Uuid::parse_str(r.get::<String, _>("asset_id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                by_asset.entry(asset_id).or_default().push(TagRow {
                    tag: r.get("tag"),
                    derived: r.get::<i64, _>("derived") != 0,
                });
            }
            for asset in chunk.iter_mut() {
                let (tags, derived) = split_tags(by_asset.remove(&asset.id).unwrap_or_default());
                asset.tags = tags;
                asset.derived_tags = derived;
            }
        }
        Ok(())
    }

    /// The SQL-side asset query: `filter` narrows, `sort` orders (`sort`'s
    /// key then `id`, ascending), `after` resumes past a keyset position, and
    /// at most `limit` rows come back with tags filled. A recursive folder
    /// scope walks `documents.parent_id` over `asset_folder` rows in a CTE.
    pub async fn query_assets(
        &self,
        world: Uuid,
        filter: &AssetFilter,
        sort: AssetSort,
        after: Option<&AssetCursor>,
        limit: u32,
    ) -> Result<Vec<Asset>, DataError> {
        let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
        if let Some(FolderFilter::In {
            folder,
            recursive: true,
        }) = filter.folder
        {
            qb.push("WITH RECURSIVE sub(id) AS (SELECT ");
            qb.push_bind(folder.to_string());
            qb.push(
                " UNION ALL SELECT d.id FROM documents d JOIN sub ON d.parent_id = sub.id                  WHERE d.doc_type = 'asset_folder') ",
            );
        }
        qb.push("SELECT a.* FROM assets a WHERE a.world_id = ");
        qb.push_bind(world.to_string());
        match filter.folder {
            None | Some(FolderFilter::Any) => {}
            Some(FolderFilter::Root) => {
                qb.push(" AND a.folder_id IS NULL");
            }
            Some(FolderFilter::In {
                folder,
                recursive: false,
            }) => {
                qb.push(" AND a.folder_id = ");
                qb.push_bind(folder.to_string());
            }
            Some(FolderFilter::In {
                recursive: true, ..
            }) => {
                qb.push(" AND a.folder_id IN (SELECT id FROM sub)");
            }
        }
        for tag in &filter.tags {
            qb.push(" AND EXISTS (SELECT 1 FROM asset_tags t WHERE t.asset_id = a.id AND t.tag = ");
            qb.push_bind(tag.clone());
            qb.push(")");
        }
        match filter.kind {
            None => {}
            Some(AssetKind::Image) => {
                qb.push(" AND a.content_type LIKE 'image/%'");
            }
            Some(AssetKind::Other) => {
                qb.push(" AND a.content_type NOT LIKE 'image/%'");
            }
        }
        if let Some(name) = &filter.name {
            // `\` escapes LIKE's own wildcards so a literal `%`/`_` in the
            // needle matches itself.
            let needle = name
                .to_lowercase()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            qb.push(" AND lower(a.original_name) LIKE '%' || ");
            qb.push_bind(needle);
            qb.push(" || '%' ESCAPE '\\'");
        }
        let key = sort.sql_key();
        if let Some(cur) = after {
            qb.push(format!(" AND ({key}, a.id) > ("));
            match sort {
                AssetSort::Name => {
                    qb.push_bind(cur.sort_key.clone());
                }
                AssetSort::Created | AssetSort::Size => {
                    let n: i64 = cur
                        .sort_key
                        .parse()
                        .map_err(|_| DataError::OpFailed("malformed cursor".into()))?;
                    qb.push_bind(n);
                }
            }
            qb.push(", ");
            qb.push_bind(cur.id.to_string());
            qb.push(")");
        }
        qb.push(format!(" ORDER BY {key}, a.id LIMIT "));
        qb.push_bind(i64::from(limit));
        let rows = qb.build().fetch_all(&self.pool).await?;
        let mut assets: Vec<Asset> = rows
            .iter()
            .map(Self::asset_from_row)
            .collect::<Result<_, _>>()?;
        self.fill_tags(&mut assets).await?;
        Ok(assets)
    }

    /// All asset rows for `world`, oldest first, tags filled.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let none = repo.list_assets_by_world(uuid::Uuid::nil()).await?;
    /// assert!(none.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_assets_by_world(&self, world: Uuid) -> Result<Vec<Asset>, DataError> {
        let rows = sqlx::query("SELECT * FROM assets WHERE world_id = ? ORDER BY created_at, id")
            .bind(world.to_string())
            .fetch_all(&self.pool)
            .await?;
        let mut assets: Vec<Asset> = rows
            .iter()
            .map(Self::asset_from_row)
            .collect::<Result<_, _>>()?;
        self.fill_tags(&mut assets).await?;
        Ok(assets)
    }
}
