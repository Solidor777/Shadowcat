// Resolves a token to its EffectiveActor: the single read-through every token-decoration
// consumer (render visual, faction border [M10b], conditions [M10c], displayName) uses.
// Linked tokens read the shared actor + apply the override whitelist; instanced tokens read
// their embedded copy. Returns null for a raw (actorless) or dangling-link token.
import type { WireDocument } from "./wire";
import type { ReadableDocuments } from "./store";
import type { ActorSystem, ActorVisual, TokenOverrides, ConditionRegistrySystem } from "./scene-docs";

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

/** Resolve a token's effective conditions to display entries (id preserved for keying), via the
 * world registry. Ids absent from the registry are dropped — a stale/garbled id yields no badge,
 * never a render error (fail-closed). The single read-through every condition consumer uses. */
export function resolveConditions(token: WireDocument, store: ReadableDocuments): { id: string; name: string; icon: string }[] {
  const eff = resolveTokenActor(token, store);
  if (!eff) return [];
  const reg = store.query("condition-registry")[0]?.system as ConditionRegistrySystem | undefined;
  const map = reg?.conditions ?? {};
  const out: { id: string; name: string; icon: string }[] = [];
  for (const id of eff.conditions) {
    const c = map[id];
    if (c) out.push({ id, name: c.name, icon: c.icon });
  }
  return out;
}

/** Where a token's conditions live + the current set. Linked tokens write the shared actor doc's
 * `/system/conditions`; instanced tokens write the embedded copy at
 * `/embedded/actor/0/system/conditions`. Returns null for a raw/dangling token. The caller gates
 * the write via `AppContext.canEdit(doc, path)` — the embedded path requires `core:manage_embedded`,
 * the actor path `core:write_fields`, so the capability model decides owner eligibility per mode. */
export interface ConditionTarget {
  doc: WireDocument;
  path: string;
  conditions: string[];
}

export function conditionTarget(token: WireDocument, store: ReadableDocuments): ConditionTarget | null {
  const sys = token.system as { actor_id?: string | null } | undefined;
  if (sys?.actor_id) {
    const actor = store.get(sys.actor_id);
    if (!actor) return null;
    return { doc: actor, path: "/system/conditions", conditions: (actor.system as ActorSystem).conditions ?? [] };
  }
  const embedded = token.embedded?.actor?.[0];
  if (embedded) {
    return { doc: token, path: "/embedded/actor/0/system/conditions", conditions: (embedded.system as ActorSystem).conditions ?? [] };
  }
  return null;
}
