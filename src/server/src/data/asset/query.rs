//! Asset query filters and ordering — the data-layer vocabulary
//! `SqliteRepository::query_assets` accepts (the HTTP layer parses query
//! strings into these; regex matching stays above the repository).
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

/// Which folders a query covers.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::query::{AssetFilter, FolderFilter};
/// use shadowcat::data::asset::{Asset, AssetMeta};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use uuid::Uuid;
///
/// # #[tokio::main]
/// # async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let world = repo.create_world("w", 0).await.unwrap();
/// let asset = Asset {
///     id: Uuid::new_v4(),
///     world_id: world.id,
///     storage_key: "w/id".into(),
///     original_name: "map.png".into(),
///     content_type: "image/webp".into(),
///     byte_size: 10,
///     created_by: None,
///     created_at: 0,
///     version: 1,
///     folder_id: None,
///     tags: vec![],
///     derived_tags: vec![],
///     meta: AssetMeta::unprocessed("image/png", 10),
/// };
/// repo.insert_asset(&asset).await.unwrap();
///
/// // The asset is filed at the world root, so `FolderFilter::Root` matches it.
/// let root_only = AssetFilter {
///     folder: Some(FolderFilter::Root),
///     ..Default::default()
/// };
/// let page = repo
///     .query_assets(world.id, &root_only, Default::default(), None, 10)
///     .await
///     .unwrap();
/// assert_eq!(page.len(), 1);
///
/// // An unrelated folder id excludes the same root-filed asset.
/// let other_folder = AssetFilter {
///     folder: Some(FolderFilter::In {
///         folder: Uuid::new_v4(),
///         recursive: false,
///     }),
///     ..Default::default()
/// };
/// let page = repo
///     .query_assets(world.id, &other_folder, Default::default(), None, 10)
///     .await
///     .unwrap();
/// assert!(page.is_empty());
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderFilter {
    /// Every asset of the world, wherever filed.
    Any,
    /// Only assets at the world root (`folder_id IS NULL`).
    Root,
    /// Assets filed directly in `folder`, or anywhere under it when `recursive`.
    In {
        /// The folder document id.
        folder: Uuid,
        /// Include every descendant folder.
        recursive: bool,
    },
}

/// Coarse content class.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::query::{AssetFilter, AssetKind};
/// use shadowcat::data::asset::{Asset, AssetMeta};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use uuid::Uuid;
///
/// # #[tokio::main]
/// # async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let world = repo.create_world("w", 0).await.unwrap();
/// let asset = Asset {
///     id: Uuid::new_v4(),
///     world_id: world.id,
///     storage_key: "w/id".into(),
///     original_name: "notes.txt".into(),
///     content_type: "text/plain".into(),
///     byte_size: 10,
///     created_by: None,
///     created_at: 0,
///     version: 1,
///     folder_id: None,
///     tags: vec![],
///     derived_tags: vec![],
///     meta: AssetMeta::unprocessed("text/plain", 10),
/// };
/// repo.insert_asset(&asset).await.unwrap();
///
/// // A `text/plain` asset does not match `AssetKind::Image`.
/// let images = AssetFilter {
///     kind: Some(AssetKind::Image),
///     ..Default::default()
/// };
/// let page = repo
///     .query_assets(world.id, &images, Default::default(), None, 10)
///     .await
///     .unwrap();
/// assert!(page.is_empty());
///
/// let others = AssetFilter {
///     kind: Some(AssetKind::Other),
///     ..Default::default()
/// };
/// let page = repo
///     .query_assets(world.id, &others, Default::default(), None, 10)
///     .await
///     .unwrap();
/// assert_eq!(page.len(), 1);
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    /// `content_type` starts with `image/`.
    Image,
    /// Everything else (pass-through uploads).
    Other,
}

/// Sort key of a query; the keyset cursor pairs it with the asset id.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::query::AssetSort;
///
/// assert_eq!(AssetSort::default(), AssetSort::Created);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssetSort {
    /// `lower(original_name)`, then id.
    Name,
    /// `created_at`, then id (the default).
    #[default]
    Created,
    /// `byte_size`, then id.
    Size,
}

impl AssetSort {
    /// The SQL expression this sort orders by.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::asset::query::AssetSort;
    ///
    /// assert_eq!(AssetSort::Name.sql_key(), "lower(a.original_name)");
    /// ```
    pub fn sql_key(self) -> &'static str {
        match self {
            AssetSort::Name => "lower(a.original_name)",
            AssetSort::Created => "a.created_at",
            AssetSort::Size => "a.byte_size",
        }
    }
}

/// The SQL-evaluated part of an asset query (regex is applied by the caller
/// over the rows this selects).
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::query::AssetFilter;
/// use shadowcat::data::asset::{Asset, AssetMeta};
/// use shadowcat::data::sqlite::SqliteRepository;
/// use uuid::Uuid;
///
/// # #[tokio::main]
/// # async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let world = repo.create_world("w", 0).await.unwrap();
/// let id = Uuid::new_v4();
/// let asset = Asset {
///     id,
///     world_id: world.id,
///     storage_key: "w/id".into(),
///     original_name: "hero.png".into(),
///     content_type: "image/webp".into(),
///     byte_size: 10,
///     created_by: None,
///     created_at: 0,
///     version: 1,
///     folder_id: None,
///     tags: vec![],
///     derived_tags: vec![],
///     meta: AssetMeta::unprocessed("image/png", 10),
/// };
/// repo.insert_asset(&asset).await.unwrap();
/// repo.set_asset_tags(id, &["hero".to_string()], &[]).await.unwrap();
///
/// let filter = AssetFilter {
///     tags: vec!["hero".into()],
///     ..Default::default()
/// };
/// let page = repo
///     .query_assets(world.id, &filter, Default::default(), None, 10)
///     .await
///     .unwrap();
/// assert_eq!(page.len(), 1, "every listed tag must be present");
///
/// let filter = AssetFilter {
///     tags: vec!["missing".into()],
///     ..Default::default()
/// };
/// let page = repo
///     .query_assets(world.id, &filter, Default::default(), None, 10)
///     .await
///     .unwrap();
/// assert!(page.is_empty());
/// # }
/// ```
#[derive(Debug, Clone, Default)]
pub struct AssetFilter {
    /// Folder scope.
    pub folder: Option<FolderFilter>,
    /// Every listed tag must be present (explicit or derived).
    pub tags: Vec<String>,
    /// Content class.
    pub kind: Option<AssetKind>,
    /// Case-insensitive substring of `original_name`.
    pub name: Option<String>,
}

/// A keyset position: the sort key and id of the last row already returned.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::query::AssetCursor;
/// use shadowcat::http::assets::query::{decode_cursor, encode_cursor};
/// use uuid::Uuid;
///
/// let cursor = AssetCursor {
///     sort_key: "5".into(),
///     id: Uuid::nil(),
/// };
/// // Round-tripping through the wire encoding recovers the same cursor.
/// let round_tripped = decode_cursor(&encode_cursor(&cursor)).unwrap();
/// assert_eq!(round_tripped, cursor);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetCursor {
    /// The sort key's textual form (`lower(name)`, or the integer as text).
    pub sort_key: String,
    /// The row id, the tiebreaker.
    pub id: Uuid,
}

/// The sort key of `asset` under `sort`, in the textual form `AssetCursor` carries.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::query::{sort_key_of, AssetSort};
/// use shadowcat::data::asset::{Asset, AssetMeta};
/// use uuid::Uuid;
///
/// let asset = Asset {
///     id: Uuid::nil(),
///     world_id: Uuid::nil(),
///     storage_key: "w/id".into(),
///     original_name: "Map.png".into(),
///     content_type: "image/png".into(),
///     byte_size: 10,
///     created_by: None,
///     created_at: 42,
///     version: 1,
///     folder_id: None,
///     tags: vec![],
///     derived_tags: vec![],
///     meta: AssetMeta::unprocessed("image/png", 10),
/// };
/// assert_eq!(sort_key_of(&asset, AssetSort::Name), "map.png");
/// assert_eq!(sort_key_of(&asset, AssetSort::Created), "42");
/// ```
pub fn sort_key_of(asset: &super::Asset, sort: AssetSort) -> String {
    match sort {
        AssetSort::Name => asset.original_name.to_lowercase(),
        AssetSort::Created => asset.created_at.to_string(),
        AssetSort::Size => asset.byte_size.to_string(),
    }
}
