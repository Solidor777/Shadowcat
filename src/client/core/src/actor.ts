// Resolves a token to its EffectiveActor: the single read-through every token-decoration
// consumer (render visual, faction border [M10b], conditions [M10c], displayName) uses.
// Linked tokens read the shared actor + apply the override whitelist; instanced tokens read
// their embedded copy. Returns null for a raw (actorless) or dangling-link token.
// Re-rooted onto the three-band document shape: `name` (the actor's real identity) lives on
// the envelope; `ActorEngine`/`TokenEngine` carry every other engine-owned field.
import type { WireDocument } from "./wire";
import type { ReadableDocuments } from "./store";
import type { ActorEngine, TokenEngine, TokenVisual, TokenOverrides, ConditionRegistryEngine, SceneEngine, VisionAssignment, RenderVisual, FaceVisual } from "./scene-docs";

export interface EffectiveActor {
  name: string | null;
  displayName: string;
  visual: TokenVisual | null;
  size: { w: number; h: number };
  shape: "square" | "circle";
  faction: string | null;
  conditions: string[];
  /** Effective vision modes for this actor/token. Per-token override replaces actor base entirely;
   * defaults to [] when neither specifies vision. */
  visionModes: VisionAssignment[];
}

function project(actorDoc: WireDocument, base: ActorEngine, overrides?: TokenOverrides | null): EffectiveActor {
  return {
    name: overrides?.name ?? actorDoc.name,
    displayName: base.displayName,
    visual: overrides?.visual ?? base.visual,
    size: overrides?.size ?? base.size,
    shape: (overrides?.shape ?? base.shape) as "square" | "circle",
    faction: base.faction,
    // Fail-closed: a missing/redacted actor `conditions` yields no conditions, never a throw in
    // the downstream `for...of` (the single chokepoint protecting every EffectiveActor consumer).
    conditions: base.conditions ?? [],
    // Override replaces actor base entirely (not merged); [] when neither present (fail-closed).
    visionModes: overrides?.vision ?? base.vision ?? [],
  };
}

/** The name to show for an actor: the real name when present, else the non-secret
 * displayName, else a generic fallback. For unauthorized recipients the server redacts the
 * real `name` to `null` (the OwnerOrGm tier), so it is null here — fail-closed: a missing
 * name yields the generic label, never a leak. The single display chokepoint every surface
 * reads. */
export function actorDisplayName(a: { name?: string | null; displayName?: string }, fallback = "Unknown Creature"): string {
  return a.name || a.displayName || fallback;
}

export function resolveTokenActor(token: WireDocument, store: ReadableDocuments): EffectiveActor | null {
  const eng = token.engine as TokenEngine | undefined;
  if (eng?.actor_id) {
    const actor = store.get(eng.actor_id);
    if (!actor) return null;
    return project(actor, actor.engine as ActorEngine, eng.overrides);
  }
  const embedded = token.embedded?.actor?.[0];
  if (embedded) return project(embedded, embedded.engine as ActorEngine);
  return null;
}

/** Resolve a token's effective conditions to display entries (id preserved for keying), via the
 * world registry. Ids absent from the registry are dropped — a stale/garbled id yields no badge,
 * never a render error (fail-closed). The single read-through every condition consumer uses. */
export function resolveConditions(token: WireDocument, store: ReadableDocuments): { id: string; name: string; icon: string }[] {
  const eff = resolveTokenActor(token, store);
  if (!eff) return [];
  const reg = store.query("condition-registry")[0]?.engine as ConditionRegistryEngine | undefined;
  const map = reg?.conditions ?? {};
  const out: { id: string; name: string; icon: string }[] = [];
  for (const id of eff.conditions) {
    const c = map[id];
    if (c) out.push({ id, name: c.name, icon: c.icon });
  }
  return out;
}

/** Where a token's conditions live + the current set. Linked tokens write the shared actor doc's
 * `/engine/conditions`; instanced tokens write the embedded copy at
 * `/embedded/actor/0/engine/conditions`. Returns null for a raw/dangling token. The caller gates
 * the write via `AppContext.canEdit(doc, path)` — the embedded path requires `core:manage_embedded`,
 * the actor path `core:write_fields`, so the capability model decides owner eligibility per mode. */
export interface ConditionTarget {
  doc: WireDocument;
  path: string;
  conditions: string[];
}

export function conditionTarget(token: WireDocument, store: ReadableDocuments): ConditionTarget | null {
  const eng = token.engine as TokenEngine | undefined;
  if (eng?.actor_id) {
    const actor = store.get(eng.actor_id);
    if (!actor) return null;
    return { doc: actor, path: "/engine/conditions", conditions: (actor.engine as ActorEngine).conditions ?? [] };
  }
  const embedded = token.embedded?.actor?.[0];
  if (embedded) {
    return { doc: token, path: "/embedded/actor/0/engine/conditions", conditions: (embedded.engine as ActorEngine).conditions ?? [] };
  }
  return null;
}

/** A token's resolved footprint in scene pixels + its shape — the single read-through the
 * renderer, hit-test, and selection ring share so they cannot diverge for multi-cell/circle
 * tokens. Actor-backed: size = EffectiveActor.size (grid units) × the parent scene's grid cell;
 * raw/dangling tokens fall back to their own transform + square. `(x,y)` is the token center.
 * Pass a pre-resolved `eff` (from a prior `resolveTokenActor` call) to avoid a second
 * resolution; omit (or pass `undefined`) to resolve internally. Pass `null` explicitly for a
 * known actorless token to skip resolution and fall back to the raw engine dimensions. */
export interface TokenBox {
  x: number;
  y: number;
  w: number;
  h: number;
  shape: "square" | "circle";
}

/** Grid cell size (px) of the token's parent scene; 100 when the scene is absent/garbled. */
function sceneCellSize(token: WireDocument, store: ReadableDocuments): number {
  const scene = token.parent_id ? store.get(token.parent_id) : undefined;
  return (scene?.engine as SceneEngine | undefined)?.grid?.size ?? 100;
}

export function resolveTokenBox(token: WireDocument, store: ReadableDocuments, eff?: EffectiveActor | null): TokenBox {
  const eng = token.engine as TokenEngine | undefined;
  const x = eng?.x ?? 0;
  const y = eng?.y ?? 0;
  const actor = eff === undefined ? resolveTokenActor(token, store) : eff;
  if (actor) {
    const cell = sceneCellSize(token, store);
    return { x, y, w: actor.size.w * cell, h: actor.size.h * cell, shape: actor.shape };
  }
  return { x, y, w: eng?.w ?? 0, h: eng?.h ?? 0, shape: "square" };
}

/** Bounding-disc radius (grid units) consumed by the M10e+ pathfinder for clearance/inflation.
 * Conservative enclosure: a square uses its half-diagonal, a circle its radius. Per-engine
 * refinement (grid clearance vs navmesh inflation) is owned by M10e/M10f. */
export function footprintRadius(eff: Pick<EffectiveActor, "shape" | "size">): number {
  const { w, h } = eff.size;
  return eff.shape === "circle" ? Math.max(w, h) / 2 : Math.hypot(w, h) / 2;
}

/** The effective face names for a token's `faces`-union visual (the actor's own faces, with any
 * per-token `overrides.visual` union projected in) — the face-swap palette's option list. Reads
 * the same `resolveTokenActor` projection `resolveTokenVisual` reads, so the palette can never
 * diverge from what actually renders. Empty when the effective visual isn't `"faces"`. */
export function selectedFaceNamesFor(token: WireDocument, store: ReadableDocuments): string[] {
  const eff = resolveTokenActor(token, store);
  return eff?.visual?.kind === "faces" ? Object.keys(eff.visual.faces) : [];
}

/** Resolve a `faces` visual to the active face's RenderVisual. Precedence: a valid manual
 * `token.engine.face` > the first `faceMap` entry whose condition id is in `conditions` (in
 * `conditions` array order — a v1 simplification, no severity ranking across simultaneously
 * active conditions) > `default` > the first key of `faces` (fail-closed continuation, never a
 * missing-visual null while any face exists). Returns null only when `faces` is empty. */
function resolveFace(
  visual: Extract<TokenVisual, { kind: "faces" }>,
  manualFace: string | null | undefined,
  conditions: string[],
): FaceVisual | null {
  const names = Object.keys(visual.faces);
  if (names.length === 0) return null;
  if (manualFace && visual.faces[manualFace]) return visual.faces[manualFace];
  if (visual.faceMap) {
    for (const id of conditions) {
      const name = visual.faceMap[id];
      if (name && visual.faces[name]) return visual.faces[name];
    }
  }
  if (visual.default && visual.faces[visual.default]) return visual.faces[visual.default];
  return visual.faces[names[0]];
}

function isValidAnimated(v: Extract<RenderVisual, { kind: "animated" }>): boolean {
  if (!Number.isFinite(v.fps) || v.fps <= 0) return false;
  if (v.source.type === "frames") return v.source.frames.length > 0;
  return Number.isInteger(v.source.rows) && v.source.rows > 0 && Number.isInteger(v.source.cols) && v.source.cols > 0;
}

/** The render boundary: resolves a token's `TokenVisual` (image, animated, or faces) down to a
 * plain `RenderVisual` (image or animated) — the only two kinds the render layer ever draws.
 * Fail-closed to `null` on any malformed/unknown shape; never throws. Pass a pre-resolved `eff`
 * to avoid a second `resolveTokenActor` call; omit to resolve internally. */
export function resolveTokenVisual(
  token: WireDocument,
  store: ReadableDocuments,
  eff?: EffectiveActor | null,
): RenderVisual | null {
  const actor = eff === undefined ? resolveTokenActor(token, store) : eff;
  const eng = token.engine as TokenEngine | undefined;
  const visual = actor?.visual ?? eng?.visual;
  if (!visual) return null;
  const resolved = visual.kind === "faces" ? resolveFace(visual, eng?.face, actor?.conditions ?? []) : visual;
  if (!resolved) return null;
  if (resolved.kind !== "image" && resolved.kind !== "animated") return null;
  if (resolved.kind === "animated" && !isValidAnimated(resolved)) return null;
  return resolved;
}
