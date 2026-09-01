//! The elevation model (2.5D, single-level): tokens and lights carry an elevation
//! above the ground plane (0 = grounded), and a wall optionally carries the elevation
//! band its occlusion applies to. Elevation only ever FILTERS which walls occlude a
//! sight/light source — the star-shaped raycast pipeline itself is untouched, and
//! sight/light ranges stay 2D (horizontal). Environment ambient light is the one
//! exception: it keeps the full wall set at every elevation (walls always shadow
//! sky-light, or daylight would flood interiors).

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use super::{vision, SceneEcs, SceneEntity};
use crate::data::engine as eng;

/// The ground plane: the elevation a token or light sits at when it authors none.
pub(crate) const GROUND: f64 = 0.0;

/// Read an optional authored elevation (`TokenEngine::elevation`,
/// `LightEngine::elevation`): absent/null reads as ground, and a non-finite
/// stored value is clamped to ground as defense in depth (unreachable through
/// JSON, but a reader must never propagate NaN into the occlusion pipeline).
pub(crate) fn elevation_or_ground(v: Option<f64>) -> f64 {
    match v {
        Some(e) if e.is_finite() => e,
        _ => GROUND,
    }
}

/// Whether a wall with elevation band `band` occludes a sight/light source at
/// elevation `e`: the wall occludes iff `bottom ≤ e ≤ top`, an absent end is
/// unbounded, and an absent band occludes every elevation. Fail-closed: a
/// malformed interval (`bottom > top`), a non-finite authored endpoint, or a
/// non-finite source elevation occludes EVERYTHING (the pre-elevation behavior),
/// so a corrupt wall or a NaN leaked past `elevation_or_ground` never opens a
/// sightline the scene did not have.
pub(crate) fn wall_occludes(band: Option<&eng::WallElevation>, e: f64) -> bool {
    if !e.is_finite() {
        return true;
    }
    let Some(b) = band else { return true };
    if b.bottom.is_some_and(|v| !v.is_finite()) || b.top.is_some_and(|v| !v.is_finite()) {
        return true;
    }
    let lo = b.bottom.unwrap_or(f64::NEG_INFINITY);
    let hi = b.top.unwrap_or(f64::INFINITY);
    if lo > hi {
        return true;
    }
    lo <= e && e <= hi
}

/// A wall segment paired with the elevation band its occlusion applies to (`None`
/// band = occludes every elevation). The collected shape both bare wall accessors
/// and their elevation-filtered `_for` variants derive from — the wall SELECTION
/// rule (doc_type, parent scene, the blocks flag) is stated once per collector, so
/// an elevation edit and a flag edit reach every consumer through the same read.
pub(crate) type BandedWall = (vision::Seg, Option<eng::WallElevation>);

/// Filter a banded wall set to the segments occluding a source at `elevation`
/// (`wall_occludes`).
pub(crate) fn walls_at_elevation(walls: &[BandedWall], elevation: f64) -> Vec<vision::Seg> {
    walls
        .iter()
        .filter(|(_, band)| wall_occludes(band.as_ref(), elevation))
        .map(|(s, _)| *s)
        .collect()
}

impl SceneEcs {
    /// The scene's `blocksSight` walls with their elevation bands (the shared collector
    /// behind `sight_walls`/`sight_walls_for`).
    pub(crate) fn sight_wall_entries(&self, scene: Uuid) -> Vec<BandedWall> {
        let mut out = Vec::new();
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            let Some(wall) = self.engine_as_cached::<eng::WallEngine>(w.doc.id, &w.doc) else {
                continue;
            };
            if wall.blocks_sight != Some(true) {
                continue;
            }
            out.push((
                vision::Seg {
                    a: (wall.seg.x1, wall.seg.y1),
                    b: (wall.seg.x2, wall.seg.y2),
                },
                wall.elevation,
            ));
        }
        out
    }

    /// The scene's `blocksLight` walls with their elevation bands (the shared collector
    /// behind `light_walls`/`light_walls_for`).
    pub(crate) fn light_wall_entries(&self, scene: Uuid) -> Vec<BandedWall> {
        let mut out = Vec::new();
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            let Some(wall) = self.engine_as_cached::<eng::WallEngine>(w.doc.id, &w.doc) else {
                continue;
            };
            if wall.blocks_light != Some(true) {
                continue;
            }
            out.push((
                vision::Seg {
                    a: (wall.seg.x1, wall.seg.y1),
                    b: (wall.seg.x2, wall.seg.y2),
                },
                wall.elevation,
            ));
        }
        out
    }

    /// The FULL `blocksSight` wall segments of `scene`, unfiltered by elevation
    /// (permission-blind: includes `gm_only` walls — a wall you cannot see still
    /// blocks your sight). Production callers are all elevation-aware
    /// (`sight_walls_for`, or `walls_at_elevation` over the banded entries); this
    /// unfiltered view remains as the test seam pinning the full-set invariant.
    #[cfg(test)]
    pub(crate) fn sight_walls(&self, scene: Uuid) -> Vec<vision::Seg> {
        self.sight_wall_entries(scene)
            .into_iter()
            .map(|(s, _)| s)
            .collect()
    }

    /// The `blocksSight` wall segments of `scene` that occlude a source at `elevation`
    /// (`wall_occludes` band test per wall).
    pub(crate) fn sight_walls_for(&self, scene: Uuid, elevation: f64) -> Vec<vision::Seg> {
        walls_at_elevation(&self.sight_wall_entries(scene), elevation)
    }

    /// The FULL `blocksLight` wall segments of `scene` (the light-occlusion geometry for
    /// the lighting mask), unfiltered by elevation. Production callers are all
    /// elevation-aware (`walls_at_elevation` over the banded entries, per emitter) except
    /// environment ambient, which keeps the full set at every elevation (it is sky-light;
    /// walls always shadow it, or daylight would flood interiors) and reads it inline in
    /// `SceneEcs::lighting_inputs_from`. This unfiltered view remains as the test seam
    /// pinning the full-set invariant.
    #[cfg(test)]
    pub(crate) fn light_walls(&self, scene: Uuid) -> Vec<vision::Seg> {
        self.light_wall_entries(scene)
            .into_iter()
            .map(|(s, _)| s)
            .collect()
    }
}

#[cfg(test)]
mod tests;
