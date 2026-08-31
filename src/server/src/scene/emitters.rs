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

use super::{engine_as, parse_hex_color, SceneEcs, SceneEntity};
use crate::data::document::Document;
use crate::data::engine as eng;
use crate::scene::lighting::{Falloff, Light};

/// Convert an engine-band emission payload at world position `pos` into the lighting field's
/// `Light`. `None` when the emission is disabled (the suppress path for a carried emission, the
/// on/off switch for a standalone light). `falloff` absent ⇒ linear (the read-side default);
/// intensity is clamped to `[0, 1]`.
fn emission_to_light(pos: (f64, f64), em: &eng::LightEmission) -> Option<Light> {
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
                    if let Some(l) = emission_to_light((le.x, le.y), &le.emission) {
                        out.push(l);
                    }
                }
                "token" => {
                    let Some(pos) = self
                        .engine_as_cached::<eng::TokenEngine>(e.doc.id, &e.doc)
                        .map(|t| (t.x, t.y))
                    else {
                        continue;
                    };
                    if let Some(l) = self
                        .token_light_emission(&e.doc)
                        .and_then(|em| emission_to_light(pos, &em))
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
                .then(a.color.cmp(&b.color))
                .then(a.intensity.total_cmp(&b.intensity))
                .then(a.bright_radius.total_cmp(&b.bright_radius))
                .then(a.dim_radius.total_cmp(&b.dim_radius))
        });
        out
    }
}
