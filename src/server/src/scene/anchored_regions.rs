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

    /// The tokens a carrier's move may have carried a following region onto or off of: every
    /// OTHER token on the carrier's scene, with its centre cell and the attached regions that
    /// cover that cell while the token's stored membership (`SceneEcs::region_membership_of`)
    /// does not name them, the regions an `Enter` is owed for. Empty when nothing follows
    /// `carrier` (`attached_regions_of`), the common case. The carrier itself is not listed: its
    /// own move or placement report already evaluates its regions at the new pose. Sorted by
    /// token id so the firing order is deterministic.
    pub(crate) fn attached_occupants(&self, carrier: Uuid) -> Vec<AttachedOccupant> {
        let attached = self.attached_regions_of(carrier);
        if attached.is_empty() {
            return Vec::new();
        }
        let Some(scene) = self
            .index
            .get(&carrier)
            .and_then(|&e| self.world.get::<&SceneEntity>(e).ok())
            .and_then(|ent| ent.doc.parent_id)
        else {
            return Vec::new();
        };
        let Some(&cell_size) = self.scene_grid_sizes().get(&scene) else {
            return Vec::new();
        };
        let grid = self.resolve_grid_shape(scene, cell_size);
        let mut tokens: Vec<Uuid> = self
            .world
            .query::<&SceneEntity>()
            .iter()
            .filter(|e| {
                e.doc.doc_type == "token" && e.doc.parent_id == Some(scene) && e.doc.id != carrier
            })
            .map(|e| e.doc.id)
            .collect();
        tokens.sort();
        let mut out = Vec::new();
        for token in tokens {
            let Some(pos) = self.token_position(token) else {
                continue;
            };
            let cell = grid.cell_of(pos);
            let elevation = self
                .index
                .get(&token)
                .and_then(|&e| self.world.get::<&SceneEntity>(e).ok())
                .and_then(|ent| self.engine_as_cached::<eng::TokenEngine>(token, &ent.doc))
                .and_then(|t| t.elevation);
            let rows = self
                .trigger_regions(
                    scene,
                    super::elevation::elevation_or_ground(elevation),
                    Some(token),
                )
                .unwrap_or_default();
            let member = self.region_membership_of(token);
            let enter_regions: Vec<Uuid> = rows
                .iter()
                .filter(|r| attached.contains(&r.region_id) && r.cells.contains(&cell))
                .filter(|r| member.is_none_or(|m| !m.contains(&r.region_id)))
                .map(|r| r.region_id)
                .collect();
            out.push(AttachedOccupant {
                scene,
                token,
                cell,
                enter_regions,
            });
        }
        out
    }
}

/// One token evaluated by `SceneEcs::attached_occupants` after a carrier moved.
pub(crate) struct AttachedOccupant {
    /// The scene shared by the carrier and the token.
    pub scene: Uuid,
    /// The token that did not move.
    pub token: Uuid,
    /// The token's centre cell, the occupancy basis for its membership diff.
    pub cell: regions::Cell,
    /// The attached regions covering `cell` whose `Enter` triggers are owed: the token's stored
    /// membership does not name them yet. Empty when every covering region was already entered.
    pub enter_regions: Vec<Uuid>,
}
