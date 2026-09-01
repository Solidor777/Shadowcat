//! Creature senses (tremorsense and kin): the `perceived` half of the vision channel.
//! Where `token_vision_floors` projects a token's vision assignments into illumination
//! floors for the terrain mask, this module projects the SAME assignments — resolved
//! through the SAME `SceneEcs::token_vision_assignments` precedence walk, so the two
//! views can never disagree on which assignments a token carries — into the grounded
//! tokens a `Perception::Creatures` mode perceives.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{BTreeSet, HashMap};

use uuid::Uuid;

use super::{elevation, eng, source_los_poly, vision, SceneEcs, SceneEntity};
use crate::data::document::{Document, WorldCapDefaults};
use crate::data::membership::PermissionContext;
use crate::data::permission::cap;

/// One scene's entry in `SceneEcs::player_perceived_tokens`'s result: the grounded tokens
/// the recipient's creature senses perceive in that scene. A scene with no perceived token
/// has NO entry — absence is the redaction, never an empty list.
pub(crate) struct PerceivedScene {
    /// Scene document id the tokens are parented to.
    pub(crate) scene: Uuid,
    /// Perceived token ids, sorted (the egress loop's change detection compares whole
    /// payloads and `hecs` iteration order is not stable).
    pub(crate) tokens: Vec<Uuid>,
}

/// A grounded creature-sense source within one scene: viewpoint, resolved senses, and the
/// LOS polygon, computed lazily on the first `requires_los` sense that needs it.
struct SenseSource {
    /// Source token id — a target equal to it is the source itself, never perceived by it.
    id: Uuid,
    /// Viewpoint in scene units.
    vp: vision::P,
    /// Source elevation (always `elevation::GROUND` — a flying source feels nothing): the
    /// wall-band filter the lazily-computed LOS polygon is raycast against.
    elevation: f64,
    /// Resolved creature senses `(range_cells, requires_los)`; a `0.0` range is unlimited.
    senses: Vec<(f64, bool)>,
    /// The source's LOS polygon, computed once when a `requires_los` sense is evaluated.
    los: Option<Vec<vision::P>>,
}

impl SceneEcs {
    /// The token's resolved creature senses: `(range_cells, requires_los)` per assignment
    /// naming a `Perception::Creatures` mode (`0.0` range = unlimited). Shares
    /// `SceneEcs::token_vision_assignments` with `token_vision_floors`, so the two can never
    /// disagree on precedence. An unknown mode id is dropped (fail-closed), as is a
    /// non-finite resolved range. An omitted assignment range inherits the mode's own
    /// `VisionMode::default_range` — the SAME inheritance rule `token_vision_floors`
    /// documents; both are authored in cells, so no per-cell conversion applies here.
    pub(crate) fn token_creature_senses(&self, token: &Document) -> Vec<(f64, bool)> {
        let modes = self.resolved_vision_modes();
        let mut out: Vec<(f64, bool)> = Vec::new();
        if let Some(assignments) = self.token_vision_assignments(token) {
            for a in assignments {
                let Some(vm) = modes.get(&a.mode) else {
                    continue;
                }; // unknown mode → drop (fail-closed)
                if vm.perceives != eng::Perception::Creatures {
                    continue;
                }
                let range = a.range.unwrap_or(vm.default_range);
                if !range.is_finite() {
                    continue;
                }
                out.push((range, vm.requires_los));
            }
        }
        out
    }

    /// The grounded tokens `ctx`'s creature senses perceive, per scene — the `perceived`
    /// half of the masked vision payload (wired in `compute_derived`). Sources are the
    /// recipient's own vision sources, gathered through the ONE admission decision
    /// (`gather_vision_sources_in_scene`, the same set the lit mask uses), kept only when
    /// grounded and carrying at least one creature sense. A target qualifies when it is
    /// grounded, in range (2D center distance in cells against the grid shape's
    /// `GridShape::world_units_per_cell`, the authored-distance scale; `0.0` = unlimited),
    /// not the perceiving source itself, and — for a `requires_los` sense only — its center
    /// lies inside the source's LOS polygon (`source_los_poly` against
    /// `SceneEcs::sight_walls_for` at the source's elevation). A `requires_los == false`
    /// sense (tremorsense) ignores walls and illumination entirely.
    ///
    /// Two gates ride on top of the geometry, neither forked:
    ///
    /// - READ: a target is named only when `ctx` holds whole-document `cap::READ` on it
    ///   (`SceneEcs::ctx_access`, the same authority the footprints channel uses) — creature
    ///   senses pierce fog, never the document permission gate.
    /// - Disjointness: a target whose center CELL is already in the recipient's
    ///   `player_lit_mask` set for the scene is not restated (the lit mask is exactly what
    ///   the recipient's `lit` payload shows, so `perceived` is disjoint from it by
    ///   construction). The mask is computed ONCE per call.
    ///
    /// Fail-closed everywhere: non-finite positions/ranges contribute nothing, a scene with
    /// no grid entry is skipped, and a degenerate LOS polygon admits nothing
    /// (`vision::point_in_poly`). Deterministic: scenes iterate sorted, token ids collect
    /// through a `BTreeSet` — the egress loop's change detection compares whole payloads
    /// and `hecs` iteration order is not stable.
    pub(crate) fn player_perceived_tokens(
        &self,
        ctx: &PermissionContext,
        world_defaults: &WorldCapDefaults,
    ) -> Vec<PerceivedScene> {
        // The recipient's terrain visibility, computed once: the exclusion set that keeps
        // `perceived` disjoint from the `lit` payload.
        let bands = self.resolved_bands();
        let lit = self.player_lit_mask(ctx.user_id, ctx.world_role, world_defaults, &bands);
        // Point-lookup only; never iterated into output, so HashMap order is inert.
        let lit_cells: HashMap<Uuid, BTreeSet<(i32, i32)>> = lit
            .into_iter()
            .map(|s| {
                (
                    s.scene,
                    s.cells.into_iter().map(|(i, j, ..)| (i, j)).collect(),
                )
            })
            .collect();

        // Scenes holding at least one token (sources and targets are both tokens), sorted.
        let mut scene_ids: Vec<Uuid> = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type == "token" {
                if let Some(sid) = e.doc.parent_id {
                    scene_ids.push(sid);
                }
            }
        }
        scene_ids.sort();
        scene_ids.dedup();

        let grid_sizes = self.scene_grid_sizes();
        let mut out: Vec<PerceivedScene> = Vec::new();
        for scene in scene_ids {
            // An absent entry means no scene document — skip rather than synthesize a grid.
            let Some(&cell) = grid_sizes.get(&scene) else {
                continue;
            };
            if !cell.is_finite() || cell <= 0.0 {
                continue;
            }
            let settings = self.resolve_scene(scene);
            let grid = self.resolve_grid_shape(scene, cell);
            // Both a sense's range and the measured distance are in cells, so the distance
            // converts through the shape's per-cell world distance, never its indexing scale.
            let world_units_per_cell = grid.world_units_per_cell();
            if !world_units_per_cell.is_finite() || world_units_per_cell <= 0.0 {
                continue;
            }
            let extent = grid.world_extent(settings.bounds);

            let mut sources: Vec<SenseSource> = Vec::new();
            for src in self.gather_vision_sources_in_scene(
                ctx.user_id,
                ctx.world_role,
                world_defaults,
                scene,
                &settings,
            ) {
                // A flying source feels nothing.
                if src.elevation != elevation::GROUND {
                    continue;
                }
                if !src.vp.0.is_finite() || !src.vp.1.is_finite() {
                    continue;
                }
                let Some(&entity) = self.index.get(&src.id) else {
                    continue;
                };
                let Ok(ent) = self.world.get::<&SceneEntity>(entity) else {
                    continue;
                };
                let senses = self.token_creature_senses(&ent.doc);
                if senses.is_empty() {
                    continue;
                }
                sources.push(SenseSource {
                    id: src.id,
                    vp: src.vp,
                    elevation: src.elevation,
                    senses,
                    los: None,
                });
            }
            if sources.is_empty() {
                continue;
            }

            let lit_set = lit_cells.get(&scene);
            let mut perceived: BTreeSet<Uuid> = BTreeSet::new();
            for e in self.world.query::<&SceneEntity>().iter() {
                let doc = &e.doc;
                if doc.doc_type != "token" || doc.parent_id != Some(scene) {
                    continue;
                }
                let Some(t) = self.engine_as_cached::<eng::TokenEngine>(doc.id, doc) else {
                    continue;
                };
                // A flying target is not felt through the ground.
                if elevation::elevation_or_ground(t.elevation) != elevation::GROUND {
                    continue;
                }
                let center = (t.x, t.y);
                if !center.0.is_finite() || !center.1.is_finite() {
                    continue;
                }
                // Already visible through the terrain mask → not restated here.
                if lit_set.is_some_and(|s| s.contains(&grid.cell_of(center))) {
                    continue;
                }
                // Creature senses pierce fog, never the READ gate: a permission-hidden
                // token is never named.
                if !self.ctx_access(ctx, world_defaults, doc).has(cap::READ) {
                    continue;
                }
                for src in &mut sources {
                    if src.id == doc.id {
                        continue;
                    }
                    let dist_cells =
                        (((center.0 - src.vp.0).powi(2) + (center.1 - src.vp.1).powi(2)).sqrt())
                            / world_units_per_cell;
                    for &(range, requires_los) in &src.senses {
                        if !(range == 0.0 || dist_cells <= range) {
                            continue;
                        }
                        if requires_los {
                            let (vp, elev) = (src.vp, src.elevation);
                            let poly = src.los.get_or_insert_with(|| {
                                source_los_poly(
                                    vp,
                                    &self.sight_walls_for(scene, elev),
                                    settings.los_restriction,
                                    extent,
                                )
                            });
                            if !vision::point_in_poly(poly, center) {
                                continue;
                            }
                        }
                        perceived.insert(doc.id);
                        break;
                    }
                }
            }
            // Redaction by absence: a scene with no perceived token has no entry.
            if !perceived.is_empty() {
                out.push(PerceivedScene {
                    scene,
                    tokens: perceived.into_iter().collect(),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
