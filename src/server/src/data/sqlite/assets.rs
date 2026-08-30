//! Asset rows + tags: the `assets` / `asset_tags` half of `SqliteRepository`,
//! in a sibling `impl` block so `sqlite.rs` stays under the file-size limit.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::*;
use crate::data::asset::{Asset, AssetMeta};

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
        let tag_rows = sqlx::query(
            "SELECT t.asset_id AS asset_id, t.tag AS tag, t.derived AS derived \
             FROM asset_tags t JOIN assets a ON a.id = t.asset_id \
             WHERE a.world_id = ? ORDER BY t.tag",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut by_asset: std::collections::HashMap<Uuid, Vec<TagRow>> =
            std::collections::HashMap::new();
        for r in tag_rows {
            let asset_id = Uuid::parse_str(r.get::<String, _>("asset_id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))?;
            by_asset.entry(asset_id).or_default().push(TagRow {
                tag: r.get("tag"),
                derived: r.get::<i64, _>("derived") != 0,
            });
        }
        for asset in &mut assets {
            if let Some(rows) = by_asset.remove(&asset.id) {
                let (tags, derived) = split_tags(rows);
                asset.tags = tags;
                asset.derived_tags = derived;
            }
        }
        Ok(assets)
    }
}
