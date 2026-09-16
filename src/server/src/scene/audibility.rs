//! The `"audibility"` derived channel's pure geometry: distance falloff, wall occlusion, stereo
//! pan, and the per-token carried `SoundEmission` resolver. Mirrors `scene::emitters`'
//! light-emitter shape exactly (`token_light_emission`/`scene_lights_excluding`) — a carried
//! sound is the SAME kind of payload (an engine-band emission resolved at the token's live
//! position), so the resolution precedence (linked actor + `overrides.sound` wholesale-replace,
//! embedded actor, raw token = no emission) is copied verbatim rather than re-derived.
//!
//! Occlusion reuses the EXACT primitives the movement gate and the sight raycaster already use
//! (`segments_cross`, `elevation::wall_occludes`) — never a second geometry rule. `falloff`/`pan`
//! are clean-room formulas (no external citation: `falloff(t) = clamp(1 - t², 0, 1)` is a
//! standard inverse-square-flavored audio rolloff shape, chosen for a smooth zero at the
//! emitter's own radius with no divide-by-zero at `t = 0`).

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

use super::{elevation, engine_as, segments_cross, vision, SceneEcs, SceneEntity};
use crate::data::document::{Document, WorldCapDefaults};
use crate::data::engine as eng;
use crate::data::membership::PermissionContext;
use crate::data::permission::cap;

/// Distance-based gain multiplier at normalized distance `t` (`distance / radius_world`, ≥ 0).
/// `1.0` at the emitter, `0.0` at and beyond its authored radius, smooth in between. A
/// non-finite `t` (a degenerate radius upstream) yields `0.0` — fail-closed to silence, never a
/// spurious full-volume emitter.
pub(crate) fn falloff(t: f64) -> f64 {
    if !t.is_finite() {
        return 0.0;
    }
    (1.0 - t * t).clamp(0.0, 1.0)
}

/// Stereo pan in `-1.0` (full left) `..=1.0` (full right): the emitter's horizontal offset from
/// the listener, scaled by the emitter's own audible radius so a source at the edge of audibility
/// pans fully to one side and a source directly overhead/underfoot (`dx == 0`) centers. A
/// non-positive or non-finite `radius_world` centers the pan (no spatial information to derive
/// it from) rather than dividing by zero.
pub(crate) fn pan_for(dx: f64, radius_world: f64) -> f64 {
    if !radius_world.is_finite() || radius_world <= 0.0 {
        return 0.0;
    }
    (dx / radius_world).clamp(-1.0, 1.0)
}

/// Whether any wall in `walls` crosses the listener–emitter segment (`segments_cross`, the SAME
/// proper-crossing/touching-endpoint test `move_walls`'s reference `blocks_move` and the sight
/// raycaster both use). `walls` is caller-filtered to the relevant elevation band
/// (`elevation::walls_at_elevation`) before this is called — this function performs no band
/// test of its own, mirroring `light_polygon`'s own division of labor (caller filters, this
/// function only crosses segments).
pub(crate) fn segment_occluded(
    listener: (f64, f64),
    emitter: (f64, f64),
    walls: &[vision::Seg],
) -> bool {
    walls
        .iter()
        .any(|w| segments_cross(listener, emitter, w.a, w.b))
}

/// `WorldSettingsEngine.audio`'s leaves resolved to their engine-literal defaults — the SAME
/// literals `AudioOverlay`'s own field docs state (spatial on, walls occlude, `0.25`
/// through-wall gain), so a missing overlay or a missing leaf within it behaves identically to
/// an explicit default-valued one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResolvedAudioOverlay {
    /// `false` short-circuits every emitter to flat channel-gain mixing (no distance/pan/
    /// occlusion) — `scene::compute_audibility`'s own `spatial` field on the payload.
    pub(crate) spatial: bool,
    /// Wall-occlusion policy.
    pub(crate) occlusion: eng::Occlusion,
    /// Gain an occluded emitter is reduced to — never silenced entirely.
    pub(crate) through_wall_gain: f64,
}

impl SceneEcs {
    /// `resolve_scene`'s counterpart for the audio overlay: reads `world-settings.audio`
    /// (`engine_of` fails closed to `AudioOverlay::default()` on a missing/malformed doc, so
    /// every leaf below is already `None` in that case) and fills each leaf's engine-literal
    /// default.
    pub(crate) fn resolve_audio_overlay(&self) -> ResolvedAudioOverlay {
        let overlay = self
            .world_settings_doc()
            .map(crate::data::engine::engine_of::<eng::WorldSettingsEngine>)
            .and_then(|w| w.audio)
            .unwrap_or_default();
        ResolvedAudioOverlay {
            spatial: overlay.spatial.unwrap_or(true),
            occlusion: overlay.occlusion.unwrap_or(eng::Occlusion::Walls),
            through_wall_gain: overlay.through_wall_gain.unwrap_or(0.25),
        }
    }

    /// The token's effective carried sound emission — line-for-line the same resolution
    /// `token_light_emission` performs for `LightEmission`, swapping `light`/`LightEmission`
    /// for `sound`/`SoundEmission`. See that function's doc for the full precedence rationale
    /// (linked-token override-or-actor, embedded-actor uncached read, raw token = `None`).
    pub(crate) fn token_sound_emission(&self, token: &Document) -> Option<eng::SoundEmission> {
        let token_eng = self.engine_as_cached::<eng::TokenEngine>(token.id, token);
        match token_eng.as_ref().and_then(|t| t.actor_id) {
            Some(id) => match self.actors.get(&id) {
                Some(actor) => token_eng
                    .as_ref()
                    .and_then(|t| t.overrides.as_ref())
                    .and_then(|o| o.sound.clone())
                    .or_else(|| {
                        self.engine_as_cached::<eng::ActorEngine>(actor.id, actor)
                            .and_then(|a| a.sound)
                    }),
                None => None,
            },
            None => token
                .embedded
                .get("actor")
                .and_then(|v| v.first())
                .and_then(engine_as::<eng::ActorEngine>)
                .and_then(|a| a.sound),
        }
    }

    /// The recipient's listening token in `scene`, per the fixed selection order: (1) `override_`
    /// when it names a token that is a token of `scene` AND the recipient holds whole-document
    /// `cap::READ` on it (an explicit `AudioListenAs` naming a token the recipient cannot even
    /// see is refused, same admission `player_perceived_tokens` applies elsewhere); (2) else the
    /// lowest-id token of `scene` the recipient OWNS (`token_effective_owner`); (3) else `None`
    /// — a GM with no override and no owned token gets no spatial listener (silence for spatial
    /// emitters; non-spatial channels — playlists, one-shots — are unaffected, since they never
    /// route through this channel).
    pub(crate) fn select_listener(
        &self,
        ctx: &PermissionContext,
        world_defaults: &WorldCapDefaults,
        scene: Uuid,
        override_: Option<Uuid>,
    ) -> Option<Uuid> {
        if let Some(id) = override_ {
            let ok = self.index.get(&id).is_some_and(|&e| {
                self.world
                    .get::<&SceneEntity>(e)
                    .ok()
                    .filter(|c| c.doc.doc_type == "token" && c.doc.parent_id == Some(scene))
                    .is_some_and(|c| self.ctx_access(ctx, world_defaults, &c.doc).has(cap::READ))
            });
            if ok {
                return Some(id);
            }
        }
        let mut owned: Vec<Uuid> = self
            .world
            .query::<&SceneEntity>()
            .iter()
            .filter(|e| e.doc.doc_type == "token" && e.doc.parent_id == Some(scene))
            .filter(|e| self.token_effective_owner(&e.doc) == Some(ctx.user_id))
            .map(|e| e.doc.id)
            .collect();
        owned.sort();
        owned.into_iter().next()
    }
}

/// One spatially-resolved carried sound emitter, ready for the client's `EmitterPlayer` to mix
/// with no further geometry — every reduction (falloff, occlusion, the authored `volume`) is
/// already folded into `gain`.
///
/// # Examples
///
/// ```
/// let emitter = shadowcat::scene::audibility::AudibleEmitter {
///     token: uuid::Uuid::new_v4(),
///     asset: "a-wind".into(),
///     gain: 0.5,
///     pan: -0.25,
///     loop_: true,
/// };
/// assert!(emitter.loop_);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "camelCase")]
pub struct AudibleEmitter {
    /// The carrying token's id — the client's per-emitter `EmitterPlayer` key (one player
    /// instance per token, restarted only when `asset` changes).
    pub token: Uuid,
    /// Asset id to play.
    pub asset: String,
    /// Final resolved gain, `0..=1`: `volume * falloff(t)`, then `* throughWallGain` if
    /// occluded under `Occlusion::Walls` (never reduced to exactly `0` by occlusion alone — a
    /// player still learns something is behind the door).
    pub gain: f64,
    /// Stereo pan, `-1..=1`.
    pub pan: f64,
    /// Loop the asset per the emission's own `loop` flag.
    #[serde(rename = "loop")]
    pub loop_: bool,
}

/// One scene's resolved slice of the `"audibility"` derived channel: the recipient's listener
/// token in THIS scene (if any) and every audible carried emitter of THIS scene, spatially
/// resolved against it. `spatial: false` short-circuits every emitter to `gain: volume, pan:
/// 0.0` — flat channel mixing, per `ResolvedAudioOverlay::spatial`'s own doc.
///
/// # Examples
///
/// ```
/// let slice = shadowcat::scene::audibility::SceneAudibility {
///     scene: uuid::Uuid::new_v4(),
///     listener: None,
///     spatial: true,
///     emitters: Vec::new(),
/// };
/// assert!(slice.emitters.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "camelCase")]
pub struct SceneAudibility {
    /// The scene this slice belongs to — lets a recipient subscribed to multiple scenes (a GM
    /// roaming independently of the party's active scene) filter to the one it renders, the
    /// SAME pattern `"footprints"`'/`"vision"`'s own per-scene tagging use.
    pub scene: Uuid,
    /// The recipient's resolved listening token in `scene`, or `None` (see `select_listener`).
    pub listener: Option<Uuid>,
    /// Whether spatial attenuation/occlusion/pan are active (`ResolvedAudioOverlay::spatial`).
    pub spatial: bool,
    /// Every enabled carried emitter of `scene`, resolved against `listener` (or, absent a
    /// listener, resolved with `gain: 0.0, pan: 0.0` — nothing to spatialize FROM, so nothing is
    /// heard; `spatial: false` overrides this to full volume regardless of `listener`).
    pub emitters: Vec<AudibleEmitter>,
}

/// The `"audibility"` derived channel's full payload: one `SceneAudibility` slice per scene
/// that parents at least one token (`SceneEcs::token_scene_ids`) AND whose scene document the
/// recipient can see (`SceneEcs::scene_visible_to`, the `ctx_can_see_engine` gate
/// `resolved_footprints` shares, so a scene id is never disclosed here that the footprints
/// channel would withhold). Mirrors `FootprintsPayload`'s `scenes: Vec<...>` wrapper shape, so a
/// recipient subscribed to (or a GM locally roaming across) more than one scene receives every
/// visible scene's audibility in one payload; the client filters to the scene it is viewing.
///
/// # Examples
///
/// ```
/// let payload = shadowcat::scene::audibility::AudibilityPayload { scenes: Vec::new() };
/// assert!(payload.scenes.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "camelCase")]
pub struct AudibilityPayload {
    /// One entry per visible scene with at least one token, in `token_scene_ids`'s
    /// deterministic order.
    pub scenes: Vec<SceneAudibility>,
}

impl SceneEcs {
    /// Compute one scene's `SceneAudibility` slice for `ctx`, given the connection's current
    /// listen-as `override_` (see `select_listener`). `world_defaults` gates
    /// `select_listener`'s override admission (`ctx_access`) the same way every other derived
    /// channel's recipient-scoping does.
    pub(crate) fn compute_audibility(
        &self,
        ctx: &PermissionContext,
        world_defaults: &WorldCapDefaults,
        scene: Uuid,
        override_: Option<Uuid>,
    ) -> SceneAudibility {
        let overlay = self.resolve_audio_overlay();
        let listener = self.select_listener(ctx, world_defaults, scene, override_);
        let cell = self.scene_grid_sizes().get(&scene).copied().unwrap_or(0.0);
        let wu_per_cell = self.resolve_grid_shape(scene, cell).world_units_per_cell();
        let sight_walls = self.sight_wall_entries(scene);

        // The listener's own position + elevation (band-filters the occlusion wall set, mirroring
        // every other per-source elevation filter in this module). Absent a listener, walls are
        // irrelevant — every emitter below already reads `listener.is_none()` and produces silence
        // before consulting them.
        let listener_geo = listener.and_then(|id| {
            let &e = self.index.get(&id)?;
            let c = self.world.get::<&SceneEntity>(e).ok()?;
            let t = self.engine_as_cached::<eng::TokenEngine>(id, &c.doc)?;
            Some(((t.x, t.y), elevation::elevation_or_ground(t.elevation)))
        });

        let mut emitters = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "token" || e.doc.parent_id != Some(scene) {
                continue;
            }
            // Unlike `token_light_emission`'s field (anonymous illumination geometry, no
            // identity attached), `AudibleEmitter.token` DIRECTLY names the emitting token —
            // so, unlike light, this is an identity disclosure and is gated on whole-document
            // `cap::READ`, the SAME admission `RecipientSight::sensed`/`player_perceived_tokens`
            // apply to creature-senses perception (a permission-hidden token's carried sound
            // must not name it to a recipient who cannot otherwise know it exists).
            if !self.ctx_access(ctx, world_defaults, &e.doc).has(cap::READ) {
                continue;
            }
            let Some(sound) = self.token_sound_emission(&e.doc) else {
                continue;
            };
            if !sound.enabled {
                continue;
            }
            let Some(t) = self.engine_as_cached::<eng::TokenEngine>(e.doc.id, &e.doc) else {
                continue;
            };
            let pos = (t.x, t.y);
            let radius_world = if sound.radius.is_finite() && sound.radius > 0.0 {
                sound.radius * wu_per_cell
            } else {
                0.0
            };
            let volume = if sound.volume.is_finite() {
                sound.volume.clamp(0.0, 1.0)
            } else {
                0.0
            };

            let (gain, pan) = if !overlay.spatial {
                (volume, 0.0)
            } else {
                match listener_geo {
                    None => (0.0, 0.0),
                    Some((lp, l_elev)) => {
                        let dx = pos.0 - lp.0;
                        let dy = pos.1 - lp.1;
                        let dist = (dx * dx + dy * dy).sqrt();
                        let t = if radius_world > 0.0 {
                            dist / radius_world
                        } else {
                            f64::INFINITY
                        };
                        let mut g = volume * falloff(t);
                        if g > 0.0 && overlay.occlusion == eng::Occlusion::Walls {
                            let walls = elevation::walls_at_elevation(&sight_walls, l_elev);
                            if segment_occluded(lp, pos, &walls) {
                                g = (g * overlay.through_wall_gain).max(0.0);
                            }
                        }
                        (g, pan_for(dx, radius_world))
                    }
                }
            };

            emitters.push(AudibleEmitter {
                token: e.doc.id,
                asset: sound.asset,
                gain,
                pan,
                loop_: sound.loop_,
            });
        }
        // Deterministic order (entity-query order is unspecified) — token id, the emitter's own
        // stable identity, needs no tie-break.
        emitters.sort_unstable_by_key(|em| em.token);

        SceneAudibility {
            scene,
            listener,
            spatial: overlay.spatial,
            emitters,
        }
    }
}

#[cfg(test)]
mod tests;
