//! Token-anchored regions: resolving a region's geometry against its carrier token.
//!
//! A region whose `RegionEngine::anchor` is set stores its shape as offsets from the carrier's
//! centre. Every consumer of region geometry (the composed `region_field`, the trigger identity
//! rows, the occluder outlines) obtains its parts through `SceneEcs::anchored_region_parts`, so the
//! translation is one shared step and the consumers cannot disagree about where a following
//! region is. Geometry stays a pure function of documents: nothing is cached against the
//! carrier's position, so a carrier move is visible to the next read.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use super::{regions, SceneEcs, SceneEntity};
use crate::data::engine as eng;

/// Whether `region` is exempt from applying to `mover`: it is anchored to `mover` and does not
/// affect its own carrier (`RegionAnchor::include_carrier` unset). The movement consumers
/// (`SceneEcs::region_field`, `SceneEcs::trigger_regions`) skip such a region for that mover;
/// lighting never asks, so an anchored ambient region still edits the carrier's own cells.
pub(super) fn exempts_mover(region: &eng::RegionEngine, mover: Option<Uuid>) -> bool {
    match (region.anchor, mover) {
        (Some(anchor), Some(mover)) => anchor.token == mover && !anchor.include_carrier,
        _ => false,
    }
}

impl SceneEcs {
    /// The carrier token's current centre and rotation (degrees), or `None` when `token` is not a
    /// `token` document parented to `scene` with a decodable engine body. `None` is what makes an
    /// anchored region DORMANT: a deleted carrier, a non-token id and a carrier moved to another
    /// scene all read the same.
    fn anchor_pose(&self, scene: Uuid, token: Uuid) -> Option<((f64, f64), f64)> {
        let &e = self.index.get(&token)?;
        let ent = self.world.get::<&SceneEntity>(e).ok()?;
        if ent.doc.doc_type != "token" || ent.doc.parent_id != Some(scene) {
            return None;
        }
        let t = self.engine_as_cached::<eng::TokenEngine>(token, &ent.doc)?;
        Some(((t.x, t.y), t.rotation))
    }

    /// The `(shape, hole)` part list of `region` in absolute scene coordinates, or `None` when the
    /// shape is malformed (`regions::parse_region_shape`) or the region is anchored to a carrier
    /// that does not resolve (`anchor_pose`; the region is dormant). A static region returns its
    /// parsed parts unchanged. The single entry point for region geometry: the composed field, the
    /// trigger rows and the occluder outlines all read it, never `parse_region_shape` directly.
    pub(crate) fn anchored_region_parts(
        &self,
        scene: Uuid,
        region: &eng::RegionEngine,
    ) -> Option<Vec<(regions::RegionShape, bool)>> {
        let parts = regions::parse_region_shape(&region.shape)?;
        let Some(anchor) = region.anchor else {
            return Some(parts);
        };
        let (pos, rotation) = self.anchor_pose(scene, anchor.token)?;
        Some(regions::anchored_parts(
            parts,
            pos,
            rotation,
            anchor.follow_rotation,
        ))
    }

    /// The ids of the enabled regions anchored to `carrier` on the carrier's own scene, sorted.
    /// Empty (the common case) when `carrier` is not a scene token or nothing follows it; callers
    /// that react to a carrier move use that as the cheap guard before any geometry work.
    #[cfg(test)]
    pub(crate) fn attached_regions_of(&self, carrier: Uuid) -> Vec<Uuid> {
        let Some(scene) = self
            .index
            .get(&carrier)
            .and_then(|&e| self.world.get::<&SceneEntity>(e).ok())
            .filter(|ent| ent.doc.doc_type == "token")
            .and_then(|ent| ent.doc.parent_id)
        else {
            return Vec::new();
        };
        let mut out: Vec<Uuid> = self
            .world
            .query::<&SceneEntity>()
            .iter()
            .filter(|e| e.doc.doc_type == "region" && e.doc.parent_id == Some(scene))
            .filter(|e| {
                self.engine_as_cached::<eng::RegionEngine>(e.doc.id, &e.doc)
                    .is_some_and(|r| r.enabled && r.anchor.is_some_and(|a| a.token == carrier))
            })
            .map(|e| e.doc.id)
            .collect();
        out.sort();
        out
    }
}
