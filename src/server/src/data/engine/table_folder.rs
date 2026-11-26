//! The `table_folder` engine document type: folder tree nodes that rollable
//! tables are filed under through `Document.parent_id`.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A table folder: a world document whose `name` is the folder name and whose
/// `parent_id` is the containing `table_folder` (`None` = world root). Tables
/// point at it through their own `parent_id`. Only ordering lives in the
/// engine band.
/// INVARIANT (enforced at the persistence chokepoint, not here): `parent_id`
/// names another `table_folder` in the same world and never forms a cycle.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::table_folder::TableFolderEngine;
///
/// let first = TableFolderEngine { sort: 0 };
/// let second = TableFolderEngine { sort: 1 };
/// assert!(first.sort < second.sort);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TableFolderEngine {
    /// Sibling sort key (ascending).
    pub sort: i64,
}
