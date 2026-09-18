//! Per-scene visibility/lighting mask computation, split out of `scene::mod` to keep that file
//! under the repository's file-size gate. Owns the shared lighting-input snapshot
//! (`LightingInputs`), the visible-cells raycast/scan shared by the movement gate and the
//! egress mask (`accumulate_visible_cells`/`cell_visible`/`source_los_poly`), the segment
//! intersection primitive every wall-crossing check in the crate shares (`segments_cross`), and
//! `compute_derived`'s per-channel dispatch. Every name here is re-exported at `scene` module
//! scope (see `scene::mod`'s `pub(crate) use`/`pub use`), so external callers still address them
//! as `scene::compute_derived`/`scene::segments_cross` — moving the implementation file never
//! moves the public path.

use super::*;

/// Scene-shared lighting/wall inputs for the visibility mask. Computed once per scene per
/// dispatch and reused for every vision source. `all_bright` short-circuits light raycasts
/// under lighting-off or globalIllumination.
pub(crate) struct LightingInputs {
    /// Skip per-light raycasts: lighting off or `GlobalIllumination`.
    pub(crate) all_bright: bool,
    /// Resolved scene lights (empty under `all_bright`).
    pub(crate) lights: Vec<lighting::Light>,
    /// Per-light visibility polygons, index-aligned with `lights` (built by mapping over it, so
    /// the lengths always agree). `visibility_polygon` unions the raycast bound's own edges into
    /// the occluder set, so a non-degenerate bound always yields a non-empty polygon; an EMPTY
    /// entry arises only from degenerate (non-finite) light positions, and `cell_illumination`
    /// reads an empty polygon as "no occluder computed" — never occludes. That fail-open is
    /// inert on this path: a position degenerate enough to empty the polygon also makes the
    /// per-cell distance non-finite, which `cell_illumination` zeroes per source.
    pub(crate) lit_polys: Vec<Vec<vision::P>>,
    /// Scene-boundary visibility polygons occluding the environment ambient (`env_light_polys`).
    /// Empty under `all_bright` (env is not the mechanism there — every LOS cell is forced bright).
    pub(crate) env_polys: Vec<Vec<vision::P>>,
    /// `blocksSight` wall segments with their elevation bands (the LOS raycast input —
    /// each vision source filters them at its own elevation through
    /// `elevation::walls_at_elevation` before raycasting).
    pub(crate) sight_walls: Vec<elevation::BandedWall>,
}

impl LightingInputs {
    /// The composed light at `point`: under `all_bright` a full-level cell (untinted with
    /// lighting off, environment-tinted under globalIllumination — level 1.0 so every vision
    /// floor, incl. normal "dim", passes and every LOS cell is visible), else the additive field
    /// (`cell_illumination_from`) over this scene's lights plus `extra` — carried lights an
    /// in-flight mover composes in at its instant position (`InstantLight`). THE one
    /// illumination read: `player_lit_mask`'s band/tint bookkeeping and `point_qualifies`'s
    /// floor test both take their `CellLight` here, so the egress mask, the movement gate and
    /// the move-stream clip cannot light a cell by different rules.
    ///
    /// `world_units_per_cell` is the shape-derived world distance of one grid step
    /// (`GridShape::world_units_per_cell`), NOT the cell indexing scale — a light's radii are
    /// authored in cells and convert through it (the two coincide on square, differ by √3 on
    /// hex).
    pub(crate) fn cell_light(
        &self,
        point: (f64, f64),
        settings: &ResolvedScene,
        world_units_per_cell: f64,
        extra: &[&InstantLight],
    ) -> crate::scene::lighting::CellLight {
        if self.all_bright {
            return crate::scene::lighting::CellLight {
                level: 1.0,
                tint: if settings.lighting_enabled {
                    settings.env_color
                } else {
                    0
                },
            };
        }
        crate::scene::lighting::cell_illumination_from(
            point,
            settings.env_intensity,
            settings.env_color,
            self.lights
                .iter()
                .enumerate()
                .map(|(k, l)| (l, self.lit_polys.get(k).map_or(&[][..], Vec::as_slice)))
                .chain(extra.iter().map(|e| (&e.light, e.occluder.as_slice()))),
            &self.env_polys,
            world_units_per_cell,
        )
    }
}

/// Whether a single sample `point` (already known to lie inside the LOS polygon) qualifies a
/// cell as visible: its composed light (`LightingInputs::cell_light`, with `extra` carried
/// lights) against `cell_visible`. This is the ONE canonical place the per-point illumination +
/// floor decision is made, shared by all three sampling arms of `visible_cells` (lenient-center,
/// lenient-corner, strict-center) and by the egress clip's `InstantSight::sees`, to prevent the
/// gate-vs-egress drift hazard: if the decision logic were inlined separately in each arm, a
/// future edit could silently fork the gate mask from the egress mask.
///
/// `world_units_per_cell` is the shape-derived world distance of one grid step
/// (`GridShape::world_units_per_cell`), NOT the cell indexing scale. Both quantities it feeds — a
/// light's radii through `cell_light`, and the vision range this function's own `dist_cells` is
/// compared against — are authored in cells, so both convert through it; the two scalars
/// coincide on square and differ by √3 on hex.
pub(crate) fn point_qualifies(
    point: (f64, f64),
    src_vp: (f64, f64),
    floors: &[(f64, f64, Option<String>)],
    settings: &ResolvedScene,
    li: &LightingInputs,
    world_units_per_cell: f64,
    extra: &[&InstantLight],
) -> bool {
    let cl = li.cell_light(point, settings, world_units_per_cell, extra);
    let dist_cells = (((point.0 - src_vp.0).powi(2) + (point.1 - src_vp.1).powi(2)).sqrt())
        / world_units_per_cell;
    cell_visible(floors, cl.level, dist_cells)
}

/// One vision source gathered by `gather_vision_sources_in_scene`: an owned or
/// observer-vision-admitted token's viewpoint + resolved vision floors. `id` is carried only for
/// `visible_cells_cached`'s
/// deterministic snapshot ordering — `visible_cells` itself never reads it.
pub(crate) struct VisSrc {
    /// Source token id (snapshot ordering only; see the struct doc).
    pub(crate) id: Uuid,
    /// Viewpoint in scene units.
    pub(crate) vp: vision::P,
    /// The source token's elevation (0 = grounded): filters the sight-wall set through
    /// `elevation::wall_occludes` and grounds tremorsense (`SceneEcs::player_perceived_tokens`).
    pub(crate) elevation: f64,
    /// Resolved vision floors: `(illumination floor, range cells, render hint)`.
    pub(crate) floors: Vec<(f64, f64, Option<String>)>,
    /// Resolved creature senses `(range_cells, requires_los)` (`token_creature_senses`) —
    /// read by `player_perceived_tokens` and the clip's `SightSource`; the lit mask ignores
    /// them, so they are no part of `VisibilityInputsSnapshot`.
    pub(crate) senses: Vec<(f64, bool)>,
}

/// One `sources` entry in `VisibilityInputsSnapshot`: `(token id, viewpoint, elevation, floors)`.
/// Elevation is part of the fingerprint: a token gaining/losing height changes which walls
/// occlude it, so the same walls at two elevations must never share a cached mask.
pub(crate) type VisSrcSnapshot = (Uuid, vision::P, f64, Vec<(f64, f64, Option<String>)>);

/// Fingerprint of every input `visible_cells`'s computation reads for one `(user, scene,
/// lenient)` call, used by `visible_cells_cached` to decide whether a prior mask may be reused.
/// Built from the SAME calls the real computation makes (`gather_vision_sources_in_scene`,
/// `resolve_scene`, `scene_grid_sizes`, `scene_lights`, and the banded wall collectors
/// `sight_wall_entries`/`light_wall_entries` — wall geometry, block flags AND elevation bands) —
/// not a
/// separately-derived "things that might matter" list — so completeness reduces to "does this
/// struct hold every field `accumulate_visible_cells`/`gather_vision_sources_in_scene` read",
/// which is directly checkable by inspection, rather than "were all mutation call sites
/// enumerated", which `engine_cache`'s `CachedEngine` already proved is an open, unboundable
/// question for this codebase (`apply_op` is not the sole mutation chokepoint). Any change to
/// what these fields hold — a token moving/changing elevation/gaining-or-losing source status,
/// a wall's blocksSight/blocksLight/geometry/elevation-band changing, a light being
/// added/moved/toggled (its `elevation` rides `lights`), a vision-mode or
/// gradation band definition changing (both flow into `sources`' `floors` via
/// `token_vision_floors`), a linked actor's vision assignment changing (same path), the scene's
/// own grid size or vision/lighting overrides changing, or world-settings' `observerVision`/
/// `losRestriction`/lighting defaults changing — is captured because it necessarily changes the
/// value of one of these fields, making the snapshot compare unequal. The inputs to source
/// ADMISSION (`user_access`'s `resolve_access_world`: the token's permissions, the caller's
/// world role, the world-level capability grants) need no fields of their own — their entire
/// effect on the mask is WHICH tokens the gathered `sources` list contains, and that list is
/// fingerprinted here.
#[derive(Clone, PartialEq)]
pub(crate) struct VisibilityInputsSnapshot {
    /// The sampling mode the mask was computed under.
    pub(crate) lenient: bool,
    /// The resolved scene settings the computation read.
    pub(crate) settings: ResolvedScene,
    /// Grid cell size in scene units.
    pub(crate) cell: f64,
    /// Every vision source's `(id, viewpoint, floors)` snapshot.
    pub(crate) sources: Vec<VisSrcSnapshot>,
    /// The scene's declared `SceneEngine::levels`: they decide each source's level
    /// (`elevation::level_of`) and therefore which level-filtered illumination field its cells
    /// are judged against, so they are fingerprinted like every other input.
    pub(crate) levels: Vec<eng::SceneLevel>,
    /// The MOVER's own resolved level id (`""` = ground/a level-less scene) — `sources` is
    /// already filtered to it, but it is fingerprinted explicitly too rather than relying
    /// solely on the filtered `sources` list to imply it.
    pub(crate) mover_level: String,
    /// Resolved scene lights.
    pub(crate) lights: Vec<lighting::Light>,
    /// `blocksLight` wall segments with their elevation bands.
    pub(crate) light_walls: Vec<elevation::BandedWall>,
    /// `blocksSight` wall segments with their elevation bands.
    pub(crate) sight_walls: Vec<elevation::BandedWall>,
}

/// `visible_cells_cache`'s per-entry value: the snapshot it was computed from, paired with the
/// mask itself.
pub(crate) type VisibleCellsCacheEntry = (
    VisibilityInputsSnapshot,
    std::collections::BTreeSet<(i32, i32)>,
);

/// The per-source LOS raycast + per-cell scan shared by `visible_cells` and
/// `visible_cells_cached` on a cache miss — the sole implementation of the expensive half of the
/// computation, so both entry points share identical behavior.
pub(crate) fn accumulate_visible_cells(
    out: &mut std::collections::BTreeSet<(i32, i32)>,
    sources: &[VisSrc],
    settings: &ResolvedScene,
    cell: f64,
    li: &LightingInputs,
    lenient: bool,
    grid: &dyn grid_shape::GridShape,
) {
    // One grid step's world distance, resolved once: it is a property of the shape, so every
    // sample of every candidate cell of every source shares the value.
    let world_units_per_cell = grid.world_units_per_cell();
    for src in sources {
        let src_walls = elevation::walls_at_elevation(&li.sight_walls, src.elevation);
        let poly = source_los_poly(
            src.vp,
            &src_walls,
            settings.los_restriction,
            grid.world_extent(settings.bounds),
        );
        if poly.len() < 3 {
            continue;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in &poly {
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
        // Lenient samples corners, so a cell just outside the center-bbox can still qualify: this
        // invocation's mode (whichever `lenient` selects) decides how much this call's OWN box is
        // padded; `scan_box_for` derives both that pad and the (always fully-padded) clamp
        // decision from the same binding, so a strict and a lenient call over the same source's
        // bbox always meet an identical window.
        let bbox = ((minx, miny), (maxx, maxy));
        let mode = if lenient {
            crate::scene::explored::ScanMode::Lenient
        } else {
            crate::scene::explored::ScanMode::Strict
        };
        let (min, max) = crate::scene::explored::scan_box_for(
            grid,
            src.vp,
            bbox,
            cell,
            crate::scene::explored::MAX_CELLS_PER_POLYGON,
            mode,
        );
        let candidates = match grid.cells_in_bounds(
            min,
            max,
            cell,
            crate::scene::explored::MAX_CELLS_PER_POLYGON,
        ) {
            Some(c) => c,
            None => {
                tracing::warn!("visible_cells scan degenerate; skipping source");
                continue;
            }
        };
        for (i, j) in candidates {
            if out.contains(&(i, j)) {
                continue;
            }
            // Strict: center only. Lenient: center first (so strict cells are always
            // included), then corners if center fails — a cell whose polygon merely clips
            // a corner still qualifies under leniency.
            let center = grid.cell_center((i, j));
            let mut found = false;
            if lenient {
                // Check center first, then corners. `cell_vertices` (the 4 square corners in
                // byte-identical order, or the 6 pointy-top hex vertices) is computed ONLY on this
                // path — the strict movement-gate mask never pays for it (6 sin/cos per hex cell).
                if vision::point_in_poly(&poly, center)
                    && point_qualifies(
                        center,
                        src.vp,
                        &src.floors,
                        settings,
                        li,
                        world_units_per_cell,
                        &[],
                    )
                {
                    found = true;
                }
                if !found {
                    let corners = grid.cell_vertices((i, j), cell);
                    for &corner in &corners {
                        if vision::point_in_poly(&poly, corner)
                            && point_qualifies(
                                corner,
                                src.vp,
                                &src.floors,
                                settings,
                                li,
                                world_units_per_cell,
                                &[],
                            )
                        {
                            found = true;
                            break;
                        }
                    }
                }
            } else {
                // Strict: center only (mirrors player_lit_mask exactly).
                if vision::point_in_poly(&poly, center)
                    && point_qualifies(
                        center,
                        src.vp,
                        &src.floors,
                        settings,
                        li,
                        world_units_per_cell,
                        &[],
                    )
                {
                    found = true;
                }
            }
            if found {
                out.insert((i, j));
            }
        }
    }
}

/// Per-cell visibility decision shared by `player_lit_mask` (egress/secrecy gate) and
/// `visible_cells` (movement gate). INVARIANT: identical for both so the move gate never
/// forbids a shipped-visible cell nor permits an unshipped one. A cell is visible iff
/// some in-range vision mode's illumination floor is met. `floors`: `(floor_min, range_cells,
/// hint)`; `range == 0.0` ⇒ unbounded. Returns false when no mode is in range (fail closed).
pub(crate) fn cell_visible(
    floors: &[(f64, f64, Option<String>)],
    cl_level: f64,
    dist_cells: f64,
) -> bool {
    let mut min_floor = f64::INFINITY;
    for (fmin, range, _hint) in floors {
        if *range == 0.0 || dist_cells <= *range {
            min_floor = min_floor.min(*fmin);
        }
    }
    min_floor.is_finite() && cl_level >= min_floor
}

/// The LOS polygon for one vision source: the raycast visibility polygon when `los_restriction`
/// is on, else the whole bound box as a rectangle (whole-scene visible). Source: raycast
/// (`vision::visibility_polygon`). `scene_extent` is the scene's WORLD-unit envelope
/// (`GridShape::world_extent` of the authored grid-unit bounds), unioned into the wall-derived
/// bound so a wall-less (or sparsely-walled) scene reveals its own full authored extent instead of
/// a degenerate `viewpoint±VISION_BOUND_MARGIN` box. THE one LOS polygon builder: `sight_sources`
/// (the fog's `player_vision_polygons`, the mover's streamed timeline and the egress clip),
/// `player_lit_mask` and `visible_cells`/`visible_cells_cached` all read it, never a forked bound
/// computation.
pub(crate) fn source_los_poly(
    vp: vision::P,
    sight_walls: &[vision::Seg],
    los_restriction: bool,
    scene_extent: grid_shape::WorldExtent,
) -> Vec<vision::P> {
    let b = vision::bound_for_scene(vp, sight_walls, scene_extent, VISION_BOUND_MARGIN);
    if los_restriction {
        vision::visibility_polygon(vp, sight_walls, b)
    } else {
        vec![
            (b.minx, b.miny),
            (b.maxx, b.miny),
            (b.maxx, b.maxy),
            (b.minx, b.maxy),
        ]
    }
}

/// Signed area ×2 of triangle abc; >0 = ccw, <0 = cw, 0 = collinear.
fn orient(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Is `p` within the axis-aligned bounding box of segment `ab` (collinearity assumed)?
fn on_segment(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> bool {
    p.0 >= a.0.min(b.0) && p.0 <= a.0.max(b.0) && p.1 >= a.1.min(b.1) && p.1 <= a.1.max(b.1)
}

/// Do segments `p1p2` and `p3p4` intersect (proper crossing or a touching endpoint /
/// T-junction)? Source: standard orientation/cross-product segment-intersection test
/// (CLRS "Determining whether two segments intersect"). A move that merely touches a wall
/// counts as blocked (conservative — a token cannot end on or graze a wall).
pub(crate) fn segments_cross(
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    p4: (f64, f64),
) -> bool {
    let d1 = orient(p3, p4, p1);
    let d2 = orient(p3, p4, p2);
    let d3 = orient(p1, p2, p3);
    let d4 = orient(p1, p2, p4);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    (d1 == 0.0 && on_segment(p3, p4, p1))
        || (d2 == 0.0 && on_segment(p3, p4, p2))
        || (d3 == 0.0 && on_segment(p1, p2, p3))
        || (d4 == 0.0 && on_segment(p1, p2, p4))
}

impl Default for SceneEcs {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a derived payload for `channel` from the scene ECS, for one
/// recipient. Returns `None` for unknown channels (→ SceneError). `ctx` is
/// accepted so vision and footprints can derive per recipient; the identity
/// payload is non-sensitive and global. `world_defaults` supplies the same
/// world-level capability grants document egress resolves READ against, so the
/// footprints channel cannot disclose a token the recipient's own document
/// stream withholds. `listen_as` is the connection's spatial-audio listening
/// override, consulted only by the `"audibility"` arm (every other channel
/// ignores it — it is passed uniformly so no channel-name branch lives at the
/// call sites).
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{WorldCapDefaults, WorldRole};
/// use shadowcat::data::membership::PermissionContext;
/// use shadowcat::scene::{compute_derived, SceneEcs};
///
/// let ecs = SceneEcs::new();
/// let ctx = PermissionContext { user_id: uuid::Uuid::new_v4(), world_role: WorldRole::Player };
/// let defaults = WorldCapDefaults::default();
/// assert!(compute_derived("not-a-real-channel", &ecs, &ctx, &defaults, None).is_none());
/// ```
pub fn compute_derived(
    channel: &str,
    ecs: &SceneEcs,
    ctx: &PermissionContext,
    world_defaults: &crate::data::document::WorldCapDefaults,
    listen_as: Option<Uuid>,
) -> Option<serde_json::Value> {
    match channel {
        // Debug seam proof (non-sensitive, global); absent in release.
        #[cfg(debug_assertions)]
        "identity" => Some(serde_json::json!({ "entity_count": ecs.entity_count() })),
        // The resolved drawn footprint of every readable token, so the client renders and
        // hit-tests the authoritative geometry instead of re-deriving it from a second formula.
        "footprints" => serde_json::to_value(ecs.resolved_footprints(ctx, world_defaults)).ok(),
        // One `SceneAudibility` slice per scene with at least one token (`token_scene_ids`)
        // that the recipient can SEE — `scene_visible_to` is the `ctx_can_see_engine` gate
        // `resolved_footprints` applies, so this channel never discloses a scene id the
        // footprints channel withholds. A GM roaming a scene independently of the party's
        // active one still receives that scene's audibility, and the client filters to the one
        // it renders. A world with no visible tokened scene yields `scenes: []`, never a
        // sentinel the client must special-case. `listen_as` is the connection's spatial-audio
        // listening override (`ClientMsg::AudioListenAs`); only this arm consults it.
        "audibility" => {
            let scenes: Vec<audibility::SceneAudibility> = ecs
                .token_scene_ids()
                .into_iter()
                .filter(|scene| ecs.scene_visible_to(ctx, world_defaults, *scene))
                .map(|scene| ecs.compute_audibility(ctx, world_defaults, scene, listen_as))
                .collect();
            serde_json::to_value(audibility::AudibilityPayload { scenes }).ok()
        }
        // Server-resolved combat resource numbers and movement budgets, per recipient — the
        // client evaluates and stores nothing (`SceneEcs::resolved_combats`'s own doc comment).
        "combat" => serde_json::to_value(ecs.resolved_combats(ctx, world_defaults)).ok(),
        // Per-player vision: the GM sees all; a player gets ONLY their own visibility
        // polygons, per-recipient. A token-less player gets empty polygons → full fog (the
        // client masks everything outside `polygons`, so empty = see nothing, never see-all).
        // Each polygon carries its `scene` so the client cuts fog holes only for the scene it
        // renders — a token in another scene must not punch a hole into the active scene's fog.
        "vision" => {
            if ctx.world_role == crate::data::document::WorldRole::Gm {
                Some(serde_json::json!({ "mode": "all" }))
            } else {
                let polygons: Vec<serde_json::Value> = ecs
                    .player_vision_polygons(ctx.user_id, ctx.world_role, world_defaults)
                    .into_iter()
                    .map(|(scene, level, poly)| {
                        let points: Vec<f64> = poly.into_iter().flat_map(|(x, y)| [x, y]).collect();
                        // `level` serializes as an empty string for ground/a level-less scene —
                        // the ONE spelling both sides use (the client's `levelOf`/`bandContains`
                        // mirrors treat `""` as the ground/default level id), never `null`.
                        serde_json::json!({ "scene": scene, "level": level, "points": points })
                    })
                    .collect();
                // The secrecy-safe lighting-aware mask — only currently-visible cells, each
                // tagged with its illumination band + tint. Carries the resolved gradation `bands`
                // so the client maps band indices → treatment. Additive: `polygons`/`explored` are
                // unchanged (the client consumes `lit` alongside them).
                // `renderHints` is a deterministic string table (first-seen order over the
                // BTreeMap-ordered mask); each cell emits 5 ints: [i,j,band,tint,hint_idx] where
                // hint_idx is the index into `renderHints`, or -1 for None.
                // The gradation is resolved ONCE here and passed into the mask computation, so
                // the payload's `bands` array and the mask's band indices are the same
                // resolution by construction.
                let bands = ecs.resolved_bands();
                let bands_json: Vec<serde_json::Value> = bands
                    .iter()
                    .map(|b| serde_json::json!({ "name": b.name, "min": b.min_illumination }))
                    .collect();
                // Build the hint table and 5-int cell packing in a plain loop to avoid a
                // mutable borrow of `hints` inside a closure/flat_map borrow conflict.
                let mask = ecs.player_lit_mask(ctx.user_id, ctx.world_role, world_defaults, &bands);
                // Creature senses (tremorsense & kin): the grounded tokens the recipient's
                // grounded sources perceive, disjoint from `lit` by construction — the SAME
                // mask value the payload's `lit` set below is built from is the exclusion
                // set (a target whose center cell is already lit is not restated). Absent on
                // the GM arm above — a GM sees all, so there is nothing to perceive.
                let perceived: Vec<serde_json::Value> = ecs
                    .player_perceived_tokens(ctx, world_defaults, &mask)
                    .into_iter()
                    .map(|p| serde_json::json!({ "scene": p.scene, "tokens": p.tokens }))
                    .collect();
                let mut hints: Vec<String> = Vec::new();
                let mut lit: Vec<serde_json::Value> = Vec::new();
                for s in mask {
                    let mut flat: Vec<i64> = Vec::new();
                    for (i, j, band, tint, hint) in s.cells {
                        let hi: i64 = match hint {
                            None => -1,
                            Some(ref h) => match hints.iter().position(|x| x == h) {
                                Some(idx) => idx as i64,
                                None => {
                                    hints.push(h.clone());
                                    (hints.len() - 1) as i64
                                }
                            },
                        };
                        flat.extend_from_slice(&[i as i64, j as i64, band as i64, tint as i64, hi]);
                    }
                    lit.push(
                        serde_json::json!({ "scene": s.scene, "level": s.level, "cell": s.cell, "cells": flat }),
                    );
                }
                Some(
                    serde_json::json!({ "mode": "masked", "polygons": polygons, "bands": bands_json, "renderHints": hints, "lit": lit, "perceived": perceived }),
                )
            }
        }
        _ => None,
    }
}
