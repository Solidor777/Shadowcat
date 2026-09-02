//! Token-carried light emissions: the resolver that joins a token to its effective
//! `LightEmission`, and the union accessor every illumination-field consumer reads.
//!
//! A carried emission is the SAME payload as a standalone light's
//! (`eng::LightEmission`, one definition) resolved at the token's live ECS position, so the
//! field's light set is standalone `light` documents ∪ carried emissions with no second read
//! path.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use super::{elevation, engine_as, parse_hex_color, vision, SceneEcs, SceneEntity};
use crate::data::document::Document;
use crate::data::engine as eng;
use crate::scene::lighting::{Falloff, Light};
use crate::scene::move_stream::LightSamplePt;

/// A light's occlusion reach in WORLD units: the larger of its finite, positive radii
/// (authored in cells) scaled by `world_units_per_cell` — the authored-distance scale, never
/// the indexing scale. Over-inclusion here is inert: `light_illumination` returns 0 beyond
/// `dim_radius` before any cell inside the polygon can light. A non-finite or non-positive
/// radius contributes nothing rather than a fallback distance.
pub(crate) fn light_reach(light: &Light, world_units_per_cell: f64) -> f64 {
    let wu = if world_units_per_cell.is_finite() && world_units_per_cell > 0.0 {
        world_units_per_cell
    } else {
        0.0
    };
    [light.bright_radius, light.dim_radius]
        .into_iter()
        .filter(|r| r.is_finite() && *r > 0.0)
        .fold(0.0_f64, f64::max)
        * wu
}

/// THE light occlusion raycast: the `blocksLight`-occluded illumination polygon of a source at
/// `pos` against `walls` (already filtered to the source's elevation band), bounded by
/// `vision::bound_for_reach` grown to `reach` world units. The committed field
/// (`SceneEcs::lighting_inputs_from`) and the carried-light move timeline
/// (`MoverLightInputs::sample_at`) both call this — never a second raycast rule.
pub(crate) fn light_polygon(pos: vision::P, walls: &[vision::Seg], reach: f64) -> Vec<vision::P> {
    let b = vision::bound_for_reach(pos, walls, super::VISION_BOUND_MARGIN, reach);
    vision::visibility_polygon(pos, walls, b)
}

/// Per-move-constant inputs for a mover's carried-light timeline, hoisted once by
/// `SceneEcs::mover_light_inputs` (the `player_vision_inputs` shape): the emission resolved
/// at the mover's elevation, the `blocksLight` walls filtered to that elevation, and the
/// scene's per-cell world distance. `sample_at` then costs one raycast per sample.
pub(crate) struct MoverLightInputs {
    /// The mover's resolved emission as a `Light`; `pos` is overwritten per sample.
    light: Light,
    /// `blocksLight` walls occluding a source at the mover's elevation.
    walls: Vec<vision::Seg>,
    /// `GridShape::world_units_per_cell` for the scene — converts the authored cell radii.
    world_units_per_cell: f64,
}

impl MoverLightInputs {
    /// The carried light raycast at `pos` (the emitter position at elapsed `t_ms`), through
    /// the committed field's own `light_polygon`; `bright`/`dim` are the authored radii in
    /// scene units (a non-finite or negative radius reads as 0 reach).
    pub(crate) fn sample_at(&self, t_ms: f64, pos: (f64, f64)) -> LightSamplePt {
        let mut light = self.light.clone();
        light.pos = pos;
        let scene_units = |r: f64| {
            if r.is_finite() && r > 0.0 {
                r * self.world_units_per_cell
            } else {
                0.0
            }
        };
        LightSamplePt {
            t_ms,
            pos,
            bright: scene_units(light.bright_radius),
            dim: scene_units(light.dim_radius),
            color: light.color,
            polygons: vec![light_polygon(
                pos,
                &self.walls,
                light_reach(&light, self.world_units_per_cell),
            )],
        }
    }
}

/// Convert an engine-band emission payload at world position `pos` into the lighting field's
/// `Light`. `None` when the emission is disabled (the suppress path for a carried emission, the
/// on/off switch for a standalone light). `falloff` absent ⇒ linear (the read-side default);
/// intensity is clamped to `[0, 1]`. `elevation` is the emitter's height above the ground plane
/// (the standalone light's own, or the carrying token's) — read through
/// `elevation::elevation_or_ground` by the caller.
fn emission_to_light(pos: (f64, f64), elevation: f64, em: &eng::LightEmission) -> Option<Light> {
    if !em.enabled {
        return None;
    }
    let falloff = match em.falloff.as_ref().map(|f| f.curve) {
        Some(eng::FalloffCurve::Quadratic) => Falloff::Quadratic,
        Some(eng::FalloffCurve::None) => Falloff::None,
        _ => Falloff::Linear,
    };
    Some(Light {
        pos,
        elevation,
        color: parse_hex_color(&em.color),
        intensity: em.intensity.clamp(0.0, 1.0),
        bright_radius: em.bright_radius,
        dim_radius: em.dim_radius,
        falloff,
        enabled: true, // INVARIANT: the `em.enabled` early return filters every disabled emission.
    })
}

impl SceneEcs {
    /// The token's effective carried light emission, resolved with the SAME precedence
    /// `token_vision_floors` implements (mirroring `resolveTokenActor`): a LINKED token
    /// (`actor_id` present) resolves the shared actor and applies `overrides.light` as a
    /// wholesale replacement when present; a dangling link (actor absent) yields `None`,
    /// ignoring overrides. An INSTANCED token (no `actor_id`) reads its embedded actor copy
    /// through the deliberately-uncached direct `engine_as` path — an embedded actor's own `id`
    /// differs from the token's, so caching under either key would go stale on an
    /// `/embedded/actor/0/...` write (the same rule `token_vision_floors`'s embedded branch
    /// follows). A raw (actorless) token carries no emission.
    ///
    /// The emitter's own visibility tier is NOT consulted: a fogged or permission-hidden
    /// token's carried light still illuminates — physically the glow precedes the bearer, and
    /// the emission is GM-authored (`enabled: false` is the suppress path). This mirrors the
    /// standing rule that a `gm_only` wall still blocks sight.
    pub(crate) fn token_light_emission(&self, token: &Document) -> Option<eng::LightEmission> {
        let token_eng = self.engine_as_cached::<eng::TokenEngine>(token.id, token);
        match token_eng.as_ref().and_then(|t| t.actor_id) {
            Some(id) => match self.actors.get(&id) {
                Some(actor) => token_eng
                    .as_ref()
                    .and_then(|t| t.overrides.as_ref())
                    .and_then(|o| o.light.clone())
                    .or_else(|| {
                        self.engine_as_cached::<eng::ActorEngine>(actor.id, actor)
                            .and_then(|a| a.light)
                    }),
                None => None, // dangling link → no emission (overrides ignored, per resolveTokenActor)
            },
            // Uncached: see this fn's doc comment for why the embedded branch must not cache.
            None => token
                .embedded
                .get("actor")
                .and_then(|v| v.first())
                .and_then(engine_as::<eng::ActorEngine>)
                .and_then(|a| a.light),
        }
    }

    /// The mover's carried-light timeline inputs for one move, or `None` when there is
    /// nothing to sample — the scene is all-bright (`ResolvedScene::all_bright`: lighting
    /// off or `GlobalIllumination`, where no light field exists), `token` is not a token
    /// entity, or its resolved emission (`token_light_emission`) is absent or disabled. This
    /// `None` is what makes the timeline "cost only on request": a lightless move performs no
    /// light raycast at all. `cell` is the scene's indexing cell size (`scene_grid_sizes`).
    pub(crate) fn mover_light_inputs(
        &self,
        scene: Uuid,
        token: Uuid,
        cell: f64,
    ) -> Option<MoverLightInputs> {
        if self.resolve_scene(scene).all_bright() {
            return None;
        }
        let &e = self.index.get(&token)?;
        let (emission, elev) = {
            let tok = self.world.get::<&SceneEntity>(e).ok()?;
            if tok.doc.doc_type != "token" || tok.doc.parent_id != Some(scene) {
                return None;
            }
            let t = self.engine_as_cached::<eng::TokenEngine>(token, &tok.doc)?;
            (
                self.token_light_emission(&tok.doc)?,
                elevation::elevation_or_ground(t.elevation),
            )
        };
        // The position is per sample; the template light carries every other field.
        let light = emission_to_light((0.0, 0.0), elev, &emission)?;
        Some(MoverLightInputs {
            light,
            walls: elevation::walls_at_elevation(&self.light_wall_entries(scene), elev),
            world_units_per_cell: self.resolve_grid_shape(scene, cell).world_units_per_cell(),
        })
    }

    /// The scene's full emitter set as `lighting::Light`s: standalone `light` documents parented
    /// to `scene` ∪ every token's resolved carried emission (`token_light_emission`) at the
    /// token's live position. Disabled emissions contribute nothing and are dropped here.
    ///
    /// THE one read path into the illumination field's light set — `lighting_inputs` and the
    /// `visible_cells_cached` snapshot both consume this, so a carried emission participates in
    /// the lit mask, the movement gate and environment composition with no second code path,
    /// and a token's move is visible to the snapshot (its `lights` entry's position changes).
    pub(crate) fn scene_lights(&self, scene: Uuid) -> Vec<Light> {
        let mut out = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.parent_id != Some(scene) {
                continue;
            }
            match e.doc.doc_type.as_str() {
                "light" => {
                    let Some(le) = self.engine_as_cached::<eng::LightEngine>(e.doc.id, &e.doc)
                    else {
                        continue;
                    };
                    if let Some(l) = emission_to_light(
                        (le.x, le.y),
                        elevation::elevation_or_ground(le.elevation),
                        &le.emission,
                    ) {
                        out.push(l);
                    }
                }
                "token" => {
                    let Some(t) = self.engine_as_cached::<eng::TokenEngine>(e.doc.id, &e.doc)
                    else {
                        continue;
                    };
                    // A carried emission emits at its token's elevation (the token IS the
                    // emitter); the token's own x/y/elevation are the position read.
                    let (pos, elev) = ((t.x, t.y), elevation::elevation_or_ground(t.elevation));
                    if let Some(l) = self
                        .token_light_emission(&e.doc)
                        .and_then(|em| emission_to_light(pos, elev, &em))
                    {
                        out.push(l);
                    }
                }
                _ => {}
            }
        }
        // Deterministic order (entity-query order is unspecified). Position alone is NOT a
        // unique key — a standalone light stacked exactly on a carrying token shares it — so
        // the chain continues through every payload field; a collision beyond that would need
        // two fully identical emissions, whose order is then genuinely irrelevant. total_cmp
        // gives a genuine total order (partial_cmp on f64 is a partial order: NaN breaks
        // trichotomy and makes sort_by non-deterministic under NaN inputs).
        out.sort_unstable_by(|a, b| {
            a.pos
                .0
                .total_cmp(&b.pos.0)
                .then(a.pos.1.total_cmp(&b.pos.1))
                .then(a.elevation.total_cmp(&b.elevation))
                .then(a.color.cmp(&b.color))
                .then(a.intensity.total_cmp(&b.intensity))
                .then(a.bright_radius.total_cmp(&b.bright_radius))
                .then(a.dim_radius.total_cmp(&b.dim_radius))
        });
        out
    }
}
