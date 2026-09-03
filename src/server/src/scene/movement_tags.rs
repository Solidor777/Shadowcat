//! Movement-type tags: the server-side resolution of a token's effective tag set and the ONE
//! reserved-semantics predicate the terrain-cost sites share. Tags themselves are inert
//! vocabulary carried on `eng::ActorEngine::movement` / `eng::Faction::movement` /
//! `eng::TokenOverrides::movement`; only the two engine-reserved tags in
//! `TERRAIN_EXEMPT_TAGS` carry engine meaning, and that meaning is narrow: difficult terrain
//! costs nothing extra — walls, impassable regions, arrest regions and the visibility mask
//! all still gate, exactly as they do for any other mover.

use std::collections::BTreeSet;

use uuid::Uuid;

use super::{engine_as, SceneEcs, SceneEntity};
use crate::data::engine as eng;

/// The engine-reserved movement tags. A mover carrying either ignores difficult-terrain COST
/// (`regions::RegionField::terrain_multiplier` reads as 1.0 at every pricing site, through the
/// single `pathfinding::terrain_cost` chokepoint) — and NOTHING else. Unknown tags are inert
/// system vocabulary (same posture as conditions), carried for system modules to interpret.
pub(crate) const TERRAIN_EXEMPT_TAGS: [&str; 2] = ["flying", "incorporeal"];

/// The ONE reserved-semantics predicate: whether this resolved tag set exempts the mover from
/// terrain cost. Both request seams (`ws::conn::handle_pathfind`,
/// `ws::room::Room::execute_move`) read exactly this — never an inline `contains("flying")` —
/// so the reserved set cannot fork across call sites.
pub(crate) fn ignores_terrain_cost(tags: &BTreeSet<String>) -> bool {
    TERRAIN_EXEMPT_TAGS.iter().any(|t| tags.contains(*t))
}

impl SceneEcs {
    /// The token's effective movement-type tags (deduplicated), resolved through the SAME
    /// linked/instanced/override precedence `SceneEcs::token_vision_assignments` implements
    /// (mirroring the client's `resolveTokenActor`), plus the faction union:
    ///
    /// - a LINKED token (`actor_id` present) resolves the shared actor: a present
    ///   `TokenOverrides::movement` REPLACES the whole set wholesale (an explicit empty array
    ///   strips every inherited tag); otherwise the actor's own `movement` unions with its
    ///   faction record's `Faction::movement`, joined through
    ///   `SceneEcs::faction_registry_engine`. A dangling ACTOR link (actor absent) yields the
    ///   empty set, overrides ignored — the same fail-closed arm `token_vision_assignments`
    ///   takes. A dangling FACTION link (the key is absent from the registry, or no registry
    ///   is hydrated) simply contributes no faction tags.
    /// - an INSTANCED token (no `actor_id`) reads its embedded actor copy through the
    ///   deliberately-uncached direct `engine_as` path — an embedded actor's own `id` differs
    ///   from the token's, so caching under either key would go stale on an
    ///   `/embedded/actor/0/...` write (the same rule `token_vision_assignments`'s embedded
    ///   branch follows). Overrides do not apply to instanced tokens; the embedded copy's
    ///   `faction` key joins the same world registry.
    /// - a raw (actorless) or unknown token id yields the empty set — fail-closed: a mover
    ///   whose tags cannot be resolved is never treated as exempt.
    pub(crate) fn token_movement_tags(&self, token: Uuid) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let Some(&e) = self.index.get(&token) else {
            return out;
        };
        let Ok(ent) = self.world.get::<&SceneEntity>(e) else {
            return out;
        };
        if ent.doc.doc_type != "token" {
            return out;
        }
        let token_eng = self.engine_as_cached::<eng::TokenEngine>(token, &ent.doc);
        // The faction union half, shared by the linked and instanced branches: the faction key
        // is a registry lookup, and every miss (no key, no registry doc, unparseable registry,
        // unknown key) contributes nothing.
        let union_faction = |faction: Option<&String>, out: &mut BTreeSet<String>| {
            let Some(key) = faction else { return };
            if let Some(f) = self
                .faction_registry_engine()
                .and_then(|reg| reg.factions.get(key).cloned())
            {
                out.extend(f.movement);
            }
        };
        match token_eng.as_ref().and_then(|t| t.actor_id) {
            Some(id) => {
                // A dangling actor link (no `actors` entry) yields the empty set, overrides
                // ignored — the same fail-closed arm `token_vision_assignments` takes.
                if let Some(actor) = self.actors.get(&id) {
                    if let Some(replacement) = token_eng
                        .as_ref()
                        .and_then(|t| t.overrides.as_ref())
                        .and_then(|o| o.movement.clone())
                    {
                        return replacement.into_iter().collect();
                    }
                    if let Some(a) = self.engine_as_cached::<eng::ActorEngine>(actor.id, actor) {
                        out.extend(a.movement);
                        union_faction(a.faction.as_ref(), &mut out);
                    }
                }
            }
            None => {
                if let Some(a) = ent
                    .doc
                    .embedded
                    .get("actor")
                    .and_then(|v| v.first())
                    .and_then(engine_as::<eng::ActorEngine>)
                {
                    out.extend(a.movement);
                    union_faction(a.faction.as_ref(), &mut out);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
