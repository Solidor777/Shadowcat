//! The `asset_folder` engine document type: folder tree nodes that
//! `assets.folder_id` points into.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An asset folder: a world document whose `name` is the folder name and whose
/// `parent_id` is the containing folder (`None` = world root). Assets point at
/// it via `assets.folder_id`. Only ordering lives in the engine band.
/// INVARIANT (enforced at the persistence chokepoint, not here): `parent_id`
/// names another `asset_folder` in the same world and never forms a cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct AssetFolderEngine {
    /// Sibling sort key (ascending).
    pub sort: i64,
}
