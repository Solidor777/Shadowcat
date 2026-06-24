// Resolves a token to its EffectiveActor: the single read-through every token-decoration
// consumer (render visual, faction border [M10b], conditions [M10c], displayName) uses.
// Linked tokens read the shared actor + apply the override whitelist; instanced tokens read
// their embedded copy. Returns null for a raw (actorless) or dangling-link token.
import type { WireDocument } from "./wire";
import type { ReadableDocuments } from "./store";
import type { ActorSystem, ActorVisual, TokenOverrides } from "./scene-docs";

export interface EffectiveActor {
  name: string;
  displayName: string;
  visual: ActorVisual;
  size: { w: number; h: number };
  shape: "square" | "circle";
  faction: string | null;
  conditions: string[];
}

function project(base: ActorSystem, overrides?: TokenOverrides): EffectiveActor {
  return {
    name: overrides?.name ?? base.name,
    displayName: base.displayName,
    visual: overrides?.visual ?? base.visual,
    size: overrides?.size ?? base.size,
    shape: base.shape,
    faction: base.faction,
    conditions: base.conditions,
  };
}

/** The name to show for an actor: the real name when present, else the non-secret
 * displayName, else a generic fallback. For unauthorized recipients the server strips the
 * real `name` (the OwnerOrGm tier), so it is absent here — fail-closed: a missing name yields
 * the generic label, never a leak. The single display chokepoint every surface reads. */
export function actorDisplayName(a: { name?: string; displayName?: string }, fallback = "Unknown Creature"): string {
  return a.name || a.displayName || fallback;
}

export function resolveTokenActor(token: WireDocument, store: ReadableDocuments): EffectiveActor | null {
  const sys = token.system as { actor_id?: string | null; overrides?: TokenOverrides } | undefined;
  if (sys?.actor_id) {
    const actor = store.get(sys.actor_id);
    if (!actor) return null;
    return project(actor.system as ActorSystem, sys.overrides);
  }
  const embedded = token.embedded?.actor?.[0];
  if (embedded) return project(embedded.system as ActorSystem);
  return null;
}
