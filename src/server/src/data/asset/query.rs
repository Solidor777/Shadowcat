//! Asset query filters and ordering — the data-layer vocabulary
//! `SqliteRepository::query_assets` accepts (the HTTP layer parses query
//! strings into these; regex matching stays above the repository).
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

/// Which folders a query covers.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    /// `content_type` starts with `image/`.
    Image,
    /// Everything else (pass-through uploads).
    Other,
}

/// Sort key of a query; the keyset cursor pairs it with the asset id.
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetCursor {
    /// The sort key's textual form (`lower(name)`, or the integer as text).
    pub sort_key: String,
    /// The row id, the tiebreaker.
    pub id: Uuid,
}

/// The sort key of `asset` under `sort`, in the textual form `AssetCursor` carries.
pub fn sort_key_of(asset: &super::Asset, sort: AssetSort) -> String {
    match sort {
        AssetSort::Name => asset.original_name.to_lowercase(),
        AssetSort::Created => asset.created_at.to_string(),
        AssetSort::Size => asset.byte_size.to_string(),
    }
}
