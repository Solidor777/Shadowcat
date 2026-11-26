//! Module-placed regions that carry an expiry.
//!
//! `Room::place_declared` stamps a region it creates from a `PlacementDecl` with a duration under
//! `system.placement.expires_at` (wall-clock milliseconds). The stamp lives in the region document,
//! so it survives a server restart: a room hydrated from persisted documents sees the same stamps,
//! and `SceneEcs::due_placement_regions` reads them straight from the scene entities with no
//! second index to rebuild or drift.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::{SceneEcs, SceneEntity};
use crate::data::document::Document;

/// The key under a placed region's `system.placement` object that holds its expiry, in wall-clock
/// milliseconds. Written by `Room::place_declared`, read by `placement_expires_at`.
pub(crate) const PLACEMENT_EXPIRES_AT_KEY: &str = "expires_at";

/// The expiry stamp of `doc` (a `region` document), or `None` when it carries none.
///
/// # Examples
///
/// ```text
/// placement_expires_at(region with system.placement.expires_at = 5000) == Some(5000)
/// placement_expires_at(region without a placement stamp) == None
/// ```
pub(crate) fn placement_expires_at(doc: &Document) -> Option<i64> {
    doc.system
        .get("placement")?
        .get(PLACEMENT_EXPIRES_AT_KEY)?
        .as_i64()
}

impl SceneEcs {
    /// The region documents whose placement expiry is at or before `now_ms`, sorted by id so one
    /// sweep commits a deterministic batch. Whole documents, because a `Delete` carries the
    /// pre-image. Empty in the common case where no placed region carries a stamp.
    pub(crate) fn due_placement_regions(&self, now_ms: i64) -> Vec<Document> {
        let mut due: Vec<Document> = self
            .world
            .query::<&SceneEntity>()
            .iter()
            .filter(|e| e.doc.doc_type == "region")
            .filter(|e| placement_expires_at(&e.doc).is_some_and(|at| at <= now_ms))
            .map(|e| e.doc.clone())
            .collect();
        due.sort_by_key(|d| d.id);
        due
    }
}
