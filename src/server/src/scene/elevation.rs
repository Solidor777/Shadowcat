//! This module is the 2.5D elevation model.
//!
//! Tokens and lights carry an elevation above the ground plane (0 = grounded), a wall optionally
//! carries the elevation band its occlusion applies to, and a scene optionally declares
//! `SceneEngine::levels` — named elevation bands (`SceneLevel`) a token's floor is derived from via
//! `level_of`. Elevation filters which walls occlude a sight/light/sound source
//! (`walls_at_elevation`), and sight/light ranges stay 2D (horizontal). Environment ambient light
//! is the one exception to that wall filter: it keeps the full wall set at every elevation (walls
//! always shadow sky-light, or daylight would flood interiors).

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

/// Whether elevation band `band` contains point `e`: `bottom ≤ e ≤ top`; an
/// absent end is unbounded, and `band: None` contains every elevation.
/// Fail-closed: a malformed interval (`bottom > top`) or a non-finite band
/// endpoint contains everything.
/// Shared by `wall_occludes` (vision/light/sound occlusion), the movement gate
/// (`SceneEcs::move_wall_entries`'s per-mover filter), `SceneEcs::region_field`'s
/// per-mover selection, `SceneEcs::trigger_regions`'s per-mover selection, and
/// the drawing/region render filters (`sceneScopedDocs`'s TS mirror
/// `bandContains`) — the never-fork pin.
pub(crate) fn band_contains(band: Option<&eng::ElevationBand>, e: f64) -> bool {
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

/// Whether an elevation-BANDED region applies to a scene level's own elevation SPAN — the
/// band-versus-band membership rule for a reader that is a whole level rather than a point
/// elevation (`band_contains`'s shape). The consumer is the per-level lighting field
/// (`SceneEcs::ambient_light_map`): a region whose band touches ANY elevation of the level's
/// `[bottom, top)` span edits that level's field. Fail-closed toward APPLYING the region (the
/// same direction `band_contains` takes on a malformed interval): an absent band applies to
/// every level; a malformed band interval or a non-finite band endpoint applies everywhere; a
/// level id the scene does not declare — and the `""` of a level-less scene — falls back to the
/// grounded point test (`band_contains(band, GROUND)`), the same answer a mover at ground reads.
pub(crate) fn band_overlaps_level(
    band: Option<&eng::ElevationBand>,
    levels: &[eng::SceneLevel],
    level: &str,
) -> bool {
    let Some(b) = band else { return true };
    if b.bottom.is_some_and(|v| !v.is_finite()) || b.top.is_some_and(|v| !v.is_finite()) {
        return true;
    }
    let lo = b.bottom.unwrap_or(f64::NEG_INFINITY);
    let hi = b.top.unwrap_or(f64::INFINITY);
    if lo > hi {
        return true;
    }
    let Some(l) = levels.iter().find(|l| l.id == level) else {
        return lo <= GROUND && GROUND <= hi;
    };
    // Closed `[lo, hi]` (the band's convention) against half-open `[bottom, top)` (the level's
    // convention): the spans share a point iff `hi >= bottom` and `lo < top` — the boundary
    // `hi == bottom` is a genuine share (both ends include it), `lo == top` is not (the level
    // excludes its top).
    hi >= l.bottom && lo < l.top
}

/// Whether a wall with elevation band `band` occludes a sight/light source at
/// elevation `e`: `band_contains(band, e)`, additionally fail-closed (occludes)
/// when `e` itself is non-finite — a corrupt wall or a NaN leaked past
/// `elevation_or_ground` never opens a sightline the scene did not have.
pub(crate) fn wall_occludes(band: Option<&eng::ElevationBand>, e: f64) -> bool {
    if !e.is_finite() {
        return true;
    }
    band_contains(band, e)
}

/// The level whose `[bottom, top)` band contains `elevation`; if none contains
/// it, the highest level whose `bottom <= elevation` (a token on a roof is on
/// the top floor); below every level's bottom, the lowest level; `levels`
/// empty, `None`. Callers pass an already-clamped elevation
/// (`elevation_or_ground`'s output), never a raw stored value. Mirrored exactly
/// by `levelOf` in `@shadowcat/core`, pinned by the shared conformance corpus
/// this module's tests read.
pub(crate) fn level_of(levels: &[eng::SceneLevel], elevation: f64) -> Option<&eng::SceneLevel> {
    if levels.is_empty() {
        return None;
    }
    if let Some(l) = levels
        .iter()
        .find(|l| l.bottom <= elevation && elevation < l.top)
    {
        return Some(l);
    }
    let mut below = levels.iter().filter(|l| l.bottom <= elevation).peekable();
    if below.peek().is_some() {
        return below.max_by(|a, b| a.bottom.partial_cmp(&b.bottom).unwrap());
    }
    levels
        .iter()
        .min_by(|a, b| a.bottom.partial_cmp(&b.bottom).unwrap())
}

/// A wall segment paired with the elevation band its occlusion applies to (`None`
/// band = occludes every elevation). The collected shape every wall accessor
/// (sight/light/sound/movement) and its elevation-filtered `_for` variant derives from — the
/// wall SELECTION rule (doc_type, parent scene, the resolved blocks flag) is stated
/// once per collector, so an elevation edit and a flag edit reach every consumer
/// through the same read.
///
/// The segment itself carries the raycast-only occlusion metadata (`vision::Seg::one_way`/
/// `vision::Seg::limited`): which collectors populate those fields (and from which
/// `WallEngine` field) differs per gate — see each collector's own doc comment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WallEntry {
    /// The wall's segment, carrying the one-way/limited occlusion metadata this gate
    /// honors (`vision::Seg::transparent_from`, `vision::nearest_hit`'s crossing budget).
    pub(crate) seg: vision::Seg,
    /// The wall's elevation band; `None` = occludes every elevation.
    pub(crate) elevation: Option<eng::ElevationBand>,
}

/// Filter a banded wall set to the segments occluding a source at `elevation`
/// (`wall_occludes`).
pub(crate) fn walls_at_elevation(walls: &[WallEntry], elevation: f64) -> Vec<vision::Seg> {
    walls
        .iter()
        .filter(|w| wall_occludes(w.elevation.as_ref(), elevation))
        .map(|w| w.seg)
        .collect()
}

impl SceneEcs {
    /// The `vision::Seg::open_within` a wall's proximity tier gives the `sense` collector, in
    /// scene units: `Some` iff `WallEngine::proximity` names `sense`. The authored cell distance
    /// converts through `GridShape::world_units_per_cell`, resolved lazily into `units_per_cell`
    /// (at most once per collector pass, and never when no wall carries a tier), so hex and
    /// square scenes agree on what "N cells" means. `gain` is the tier's `sound_gain`, `1.0`
    /// when absent.
    fn proximity_open(
        &self,
        scene: Uuid,
        wall: &eng::WallEngine,
        sense: eng::WallSense,
        units_per_cell: &mut Option<f64>,
    ) -> Option<vision::OpenWithin> {
        let tier = wall.proximity.as_ref()?;
        if !tier.senses.contains(&sense) {
            return None;
        }
        let wu = *units_per_cell.get_or_insert_with(|| {
            let cell = self.scene_grid_sizes().get(&scene).copied().unwrap_or(0.0);
            self.resolve_grid_shape(scene, cell).world_units_per_cell()
        });
        Some(vision::OpenWithin {
            distance: tier.within * wu,
            gain: tier.sound_gain.unwrap_or(1.0),
        })
    }

    /// The wall entries the scene's enabled regions contribute for `sense`: one entry per outline
    /// segment of every region whose `RegionEngine::occludes` flags `sense`
    /// (`regions::occluder_outline`, so the region's coverage is the one `rasterize_compound`
    /// derivation movement and ambient light read). Appended by `sight_wall_entries`,
    /// `light_wall_entries` and `sound_wall_entries`, so every raycast, the visibility cache
    /// fingerprint and the audibility test see one set. Entries are symmetric and immediate
    /// (`one_way: None`, `limited: false`, `open_within: None`) and carry the region's
    /// `elevation` band, so `walls_at_elevation` filters them exactly like a wall's. The
    /// authority is the permission-blind view (a `gm_only` region still occludes for every
    /// viewer, like a wall you cannot see). Movement collectors never include them.
    /// Outlines are memoized per region in `SceneEcs::occluder_cache`; a scene with no
    /// occluding region does no work beyond the document scan.
    pub(crate) fn region_occluder_entries(
        &self,
        scene: Uuid,
        sense: eng::WallSense,
    ) -> Vec<WallEntry> {
        let mut out = Vec::new();
        let mut resolved: Option<(
            f64,
            super::GridKind,
            Box<dyn super::grid_shape::GridShape + Send + Sync>,
        )> = None;
        for e in self.world.query::<&SceneEntity>().iter() {
            let doc = &e.doc;
            if doc.doc_type != "region" || doc.parent_id != Some(scene) {
                continue;
            }
            let Some(region) = self.engine_as_cached::<eng::RegionEngine>(doc.id, doc) else {
                continue;
            };
            if !region.enabled || !region.occludes.is_some_and(|o| o.blocks(sense)) {
                continue;
            }
            let Some(parts) = self.anchored_region_parts(scene, &region) else {
                continue;
            };
            if resolved.is_none() {
                let Some(cell) = self.scene_grid_sizes().get(&scene).copied() else {
                    return out;
                };
                resolved = Some((
                    cell,
                    self.resolve_grid_kind(scene),
                    self.resolve_grid_shape(scene, cell),
                ));
            }
            let Some((cell, kind, grid)) = resolved.as_ref() else {
                continue;
            };
            let outline = {
                let mut cache = self.occluder_cache.lock().unwrap();
                match cache.get(&doc.id) {
                    Some(hit)
                        if hit.parts == parts
                            && hit.cell.to_bits() == cell.to_bits()
                            && hit.kind == *kind =>
                    {
                        hit.outline.clone()
                    }
                    _ => {
                        let outline = std::sync::Arc::new(super::regions::occluder_outline(
                            &parts, *cell, &**grid,
                        ));
                        cache.insert(
                            doc.id,
                            super::regions::OccluderCacheEntry {
                                parts: parts.clone(),
                                cell: *cell,
                                kind: *kind,
                                outline: outline.clone(),
                            },
                        );
                        outline
                    }
                }
            };
            out.extend(outline.iter().map(|&(a, b)| WallEntry {
                seg: vision::Seg {
                    a,
                    b,
                    one_way: None,
                    limited: false,
                    open_within: None,
                },
                elevation: region.elevation,
            }));
        }
        out
    }

    /// The scene's `blocksSight` walls with their elevation bands (the shared collector
    /// behind `sight_walls`/`sight_walls_for`). The flag read goes through
    /// `WallEngine::effective_flags` — the ONE resolver over the authored flags and the
    /// wall's door state — so an open door occludes nothing even when every authored flag
    /// is set. The segment ALSO carries the sight gates' occlusion metadata:
    /// `vision::Seg::one_way` from `WallEngine::direction` (a one-way wall occludes only
    /// from its blocking side — `vision::Seg::transparent_from`) and
    /// `vision::Seg::limited` from `WallEngine::sight_tier` (a sight ray crosses one such
    /// wall per ray — `vision::MAX_LIMITED_CROSSINGS`), and `vision::Seg::open_within` when
    /// `WallEngine::proximity` names `WallSense::Sight`.
    pub(crate) fn sight_wall_entries(&self, scene: Uuid) -> Vec<WallEntry> {
        let mut out = Vec::new();
        let mut units_per_cell = None;
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            let Some(wall) = self.engine_as_cached::<eng::WallEngine>(w.doc.id, &w.doc) else {
                continue;
            };
            if !wall.effective_flags().blocks_sight {
                continue;
            }
            out.push(WallEntry {
                seg: vision::Seg {
                    a: (wall.seg.x1, wall.seg.y1),
                    b: (wall.seg.x2, wall.seg.y2),
                    one_way: wall.direction.map(|d| d.blocking_side),
                    limited: wall.sight_tier == Some(eng::WallSightTier::Limited),
                    open_within: self.proximity_open(
                        scene,
                        &wall,
                        eng::WallSense::Sight,
                        &mut units_per_cell,
                    ),
                },
                elevation: wall.elevation,
            });
        }
        out.extend(self.region_occluder_entries(scene, eng::WallSense::Sight));
        out
    }

    /// The scene's `blocksLight` walls with their elevation bands (the shared collector
    /// behind `light_walls`/`light_walls_for`). The flag read goes through
    /// `WallEngine::effective_flags` — the ONE resolver over the authored flags and the
    /// wall's door state — so an open door stops no light even when every authored flag is
    /// set. Light occlusion is SYMMETRIC and immediate by design: `one_way`/`limited` are
    /// always `None`/`false` here regardless of the wall's authored `direction`/`sight_tier`
    /// (those are sight/movement gates only — see `WallEngine::direction`'s own doc).
    /// `vision::Seg::open_within` is filled when the wall's `WallEngine::proximity` names
    /// `WallSense::Light`.
    pub(crate) fn light_wall_entries(&self, scene: Uuid) -> Vec<WallEntry> {
        let mut out = Vec::new();
        let mut units_per_cell = None;
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            let Some(wall) = self.engine_as_cached::<eng::WallEngine>(w.doc.id, &w.doc) else {
                continue;
            };
            if !wall.effective_flags().blocks_light {
                continue;
            }
            out.push(WallEntry {
                seg: vision::Seg {
                    a: (wall.seg.x1, wall.seg.y1),
                    b: (wall.seg.x2, wall.seg.y2),
                    one_way: None,
                    limited: false,
                    open_within: self.proximity_open(
                        scene,
                        &wall,
                        eng::WallSense::Light,
                        &mut units_per_cell,
                    ),
                },
                elevation: wall.elevation,
            });
        }
        out.extend(self.region_occluder_entries(scene, eng::WallSense::Light));
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
            .map(|w| w.seg)
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
            .map(|w| w.seg)
            .collect()
    }

    /// The scene's sound-blocking walls with their elevation bands (the shared collector
    /// behind `sound_walls_for`). The flag read goes through
    /// `WallEngine::effective_flags` — the ONE resolver over the authored flags and the
    /// wall's door state — reading its `blocks_sound` output: an explicitly authored
    /// `blocksSound` always wins, an absent one inherits the door-resolved `blocks_sight`,
    /// and an open door occludes no sound regardless of either. Sound occlusion is
    /// SYMMETRIC and immediate by design: `one_way`/`limited` are always `None`/`false`
    /// here regardless of the wall's authored `direction`/`sight_tier` (those are
    /// sight/movement gates only — see `WallEngine::direction`'s own doc), exactly as
    /// `light_wall_entries` treats them. `vision::Seg::open_within` is filled when the wall's
    /// `WallEngine::proximity` names `WallSense::Sound`, carrying its `sound_gain`.
    pub(crate) fn sound_wall_entries(&self, scene: Uuid) -> Vec<WallEntry> {
        let mut out = Vec::new();
        let mut units_per_cell = None;
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            let Some(wall) = self.engine_as_cached::<eng::WallEngine>(w.doc.id, &w.doc) else {
                continue;
            };
            if !wall.effective_flags().blocks_sound {
                continue;
            }
            out.push(WallEntry {
                seg: vision::Seg {
                    a: (wall.seg.x1, wall.seg.y1),
                    b: (wall.seg.x2, wall.seg.y2),
                    one_way: None,
                    limited: false,
                    open_within: self.proximity_open(
                        scene,
                        &wall,
                        eng::WallSense::Sound,
                        &mut units_per_cell,
                    ),
                },
                elevation: wall.elevation,
            });
        }
        out.extend(self.region_occluder_entries(scene, eng::WallSense::Sound));
        out
    }

    /// The sound-blocking wall segments of `scene` that occlude a listener at `elevation`
    /// (`wall_occludes` band test per wall) — the wall set
    /// `scene::audibility::compute_audibility`'s occlusion test runs against.
    pub(crate) fn sound_walls_for(&self, scene: Uuid, elevation: f64) -> Vec<vision::Seg> {
        walls_at_elevation(&self.sound_wall_entries(scene), elevation)
    }
}

#[cfg(test)]
mod tests;
