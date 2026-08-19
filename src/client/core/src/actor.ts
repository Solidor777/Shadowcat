// Resolves a token to its EffectiveActor: the single read-through every token-decoration
// consumer (render visual, faction border, conditions, displayName) uses.
// Linked tokens read the shared actor + apply the override whitelist; instanced tokens read
// their embedded copy. Returns null for a raw (actorless) or dangling-link token.
// Re-rooted onto the three-band document shape: `name` (the actor's real identity) lives on
// the envelope; `ActorEngine`/`TokenEngine` carry every other engine-owned field.
import type { WireDocument } from "./wire";
import type { ReadableDocuments } from "./store";
import type { ActorEngine, TokenEngine, TokenVisual, TokenOverrides, ConditionRegistryEngine, VisionAssignment, RenderVisual, FaceVisual } from "./scene-docs";
import type { FootprintLookup } from "./footprints";

/** The projected, display-ready shape every token-decoration consumer reads: a per-token
 * `TokenOverrides` whitelist folded onto its actor's `ActorEngine` base (see `project`). */
export interface EffectiveActor {
  /** The actor's real, privacy-gateable name (envelope field); `null` when unset or redacted by
   * the server for this recipient (the `OwnerOrGm` tier — see `setNameHidden`). */
  name: string | null;
  /** The actor's non-secret fallback name, always visible regardless of `name`'s redaction. */
  displayName: string;
  /** The effective render visual; `null` when neither the token override nor the actor base sets
   * one (resolve further via `resolveTokenVisual`, which also handles the `"faces"` union). */
  visual: TokenVisual | null;
  /** Authored footprint size in grid units — cells on a square grid, hexes on a hex grid. The
   * server resolves it into the drawn extent `resolveTokenBox` reads off the wire; nothing on the
   * client converts it. */
  size: {
    /** Width, grid units. */
    w: number;
    /** Height, grid units. */
    h: number;
  };
  /** The footprint shape used for rendering and hit-testing (a circle token picks by ellipse
   * containment, any other shape by its bounding box). Purely a client-side shape: the server's
   * own collision disc reads the same authored value independently. */
  shape: "square" | "circle";
  /** The assigned faction's id, or `null` for no faction. Resolve the `Faction` record itself via
   * the world's `FactionRegistryEngine`. */
  faction: string | null;
  /** Raw effective condition ids (not display entries); resolve badges via `resolveConditions`. */
  conditions: string[];
  /** Effective vision modes for this actor/token. Per-token override replaces actor base entirely;
   * defaults to [] when neither specifies vision. */
  visionModes: VisionAssignment[];
}

/** Fold a per-token `TokenOverrides` whitelist onto its actor's `ActorEngine` base to produce the
 * effective, display-ready shape. Not exported (folded into `resolveTokenActor`'s public surface).
 * @param actorDoc The resolved actor document (linked live, or the token's embedded copy).
 * @param base The actor's `engine` body.
 * @param overrides The token's own override whitelist, if any (absent for an embedded/instanced actor).
 * @returns The projected `EffectiveActor`.
 * @example
 * ```
 * // internal helper; not part of the public API (see resolveTokenActor for the public entry point)
 * declare const actorDoc: WireDocument;
 * declare const token: WireDocument;
 * project(actorDoc, actorDoc.engine as ActorEngine, (token.engine as TokenEngine | undefined)?.overrides);
 * ```
 */
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
 * reads.
 * @param a An object carrying the resolved `name`/`displayName` (typically an `EffectiveActor`).
 * @param a.name The real name, or `null`/absent when unset or redacted.
 * @param a.displayName The non-secret fallback name.
 * @param fallback The label to show when neither `name` nor `displayName` is set.
 * @returns `a.name`, else `a.displayName`, else `fallback`.
 * @example
 * ```ts
 * import { actorDisplayName } from "@shadowcat/core";
 *
 * actorDisplayName({ name: null, displayName: "the Hooded Figure" }); // "the Hooded Figure"
 * actorDisplayName({ name: null, displayName: "" }); // "Unknown Creature"
 * ```
 */
export function actorDisplayName(a: { /** The real name, or `null`/absent when unset or redacted. */ name?: string | null; /** The non-secret fallback name. */ displayName?: string }, fallback = "Unknown Creature"): string {
  return a.name || a.displayName || fallback;
}

/** The single read-through every token-decoration consumer (render visual, faction border,
 * conditions, display name) uses. Checks `token.engine.actor_id` FIRST: when set, the token is
 * linked and the shared actor is resolved live from `store` (a dangling link — `actor_id` set but
 * absent from `store` — returns `null`, it does NOT fall back to any embedded copy). Only when
 * there is no `actor_id` at all does it fall back to `token.embedded.actor[0]` (an instanced
 * token's frozen copy). A raw token (neither) returns `null`.
 * @param token The token document to resolve.
 * @param store The document store to resolve a linked actor against.
 * @returns The projected `EffectiveActor`, or `null` for a raw or dangling-link token.
 * @example
 * ```ts
 * import { resolveTokenActor, type ReadableDocuments, type WireDocument } from "@shadowcat/core";
 *
 * declare const token: WireDocument;
 * declare const store: ReadableDocuments;
 * const eff = resolveTokenActor(token, store);
 * eff?.displayName;
 * ```
 */
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

/**
 * The user a document effectively belongs to — the client mirror of the server's
 * `data::permission::effective_owner`, which is the authority. `doc.owner` is the
 * explicit per-document override; a `token` with no override inherits its LINKED actor's
 * owner, resolved live from the store so re-assigning an actor re-owns its tokens with no
 * re-stamp.
 *
 * Mirrors the server's PRECEDENCE (token's own `/owner`, else the linked actor's owner) but
 * OMITS the server's `actor.scope === doc.scope` guard (`effective_owner`
 * rejects a resolved actor whose `scope` differs from the token's) — `store.get(actorId)` here
 * is a plain id lookup with no scope filter. This is safe only because the client's
 * `DocumentStore` never holds a cross-scope document today (it is fed solely by the single
 * connected world's WS stream; a `"compendium"`-scoped id never enters `store`), not because
 * the check is unnecessary in principle.
 * TODO: enforce the scope guard structurally instead of relying on `store`'s current feed shape.
 *
 * Fail-closed: no link, a dangling link, a resolved document that is not an actor, and an
 * unowned actor all yield `null`. INSTANCED tokens deliberately do NOT inherit from their
 * embedded `actor[0]` copy (unlike `resolveTokenActor`, which reads it for display): that copy
 * is a frozen placement-time snapshot, so inheriting from it would be the stamped semantics
 * this rule exists to avoid. An instanced/raw token uses its own `owner` override.
 *
 * Advisory ONLY, like every other client capability mirror — the server re-resolves this
 * against its own transaction and rejects a bypass.
 * @param doc The document to resolve effective ownership for.
 * @param store The document store to resolve a linked actor against (token doc_type only).
 * @returns The effective owner's user id, or `null` (no owner / dangling link / non-owned actor).
 * @example
 * ```ts
 * import { effectiveOwner, type ReadableDocuments, type WireDocument } from "@shadowcat/core";
 *
 * declare const token: WireDocument;
 * declare const store: ReadableDocuments;
 * effectiveOwner(token, store); // token.owner if set, else the linked actor's owner
 * ```
 */
export function effectiveOwner(doc: WireDocument, store: ReadableDocuments): string | null {
  if (doc.owner) return doc.owner;
  if (doc.doc_type !== "token") return null;
  const actorId = (doc.engine as TokenEngine | undefined)?.actor_id;
  if (!actorId) return null;
  const actor = store.get(actorId);
  if (!actor || actor.doc_type !== "actor") return null;
  return actor.owner ?? null;
}

/**
 * Whether `userId` holds the `DocRole.Owner` capability floor on `doc` by virtue of
 * effective ownership. Mirrors the server's `data::permission::effective_role`,
 * INCLUDING its token scoping: the floor applies to `token` documents only — on every
 * other doc_type `owner` stays provenance-only and grants no capability. Keeping the
 * scoping here (not at the call site) is what stops the client gate from drifting open
 * relative to the server.
 * @param doc The document to test.
 * @param userId The user id to test for the effective-owner floor.
 * @param store The document store to resolve a linked actor against.
 * @returns `true` iff `doc.doc_type === "token"` and `effectiveOwner(doc, store) === userId`.
 * @example
 * ```ts
 * import { ownerFloorApplies, type ReadableDocuments, type WireDocument } from "@shadowcat/core";
 *
 * declare const token: WireDocument;
 * declare const store: ReadableDocuments;
 * ownerFloorApplies(token, "user-1", store);
 * ```
 */
export function ownerFloorApplies(doc: WireDocument, userId: string, store: ReadableDocuments): boolean {
  if (doc.doc_type !== "token") return false;
  return effectiveOwner(doc, store) === userId;
}

/** A condition badge ready for display: the registry-resolved `name`/`icon` alongside the raw
 * `id` (kept for stable keying in a list, since `name`/`icon` alone are not guaranteed unique). */
interface ConditionDisplayEntry {
  /** The effective condition id (matches a key in the world's `ConditionRegistryEngine.conditions`). */
  id: string;
  /** The registry's display name for `id` at resolution time. */
  name: string;
  /** The registry's emoji glyph for `id` at resolution time. */
  icon: string;
}

/** Resolve a token's effective conditions to display entries (id preserved for keying), via the
 * world registry. Ids absent from the registry are dropped — a stale/garbled id yields no badge,
 * never a render error (fail-closed). The single read-through every condition consumer uses.
 * @param token The token to resolve effective conditions for.
 * @param store The document store to resolve the actor + condition registry against.
 * @returns Display entries `{id, name, icon}`, one per effective condition id that IS present in
 * the world's condition registry (an unregistered id is dropped, not the whole list); `[]` for a
 * raw/dangling token, or when none of the token's condition ids are registered.
 * @example
 * ```ts
 * import { resolveConditions, type ReadableDocuments, type WireDocument } from "@shadowcat/core";
 *
 * declare const token: WireDocument;
 * declare const store: ReadableDocuments;
 * resolveConditions(token, store); // [{ id: "prone", name: "Prone", icon: "..." }, ...]
 * ```
 */
export function resolveConditions(token: WireDocument, store: ReadableDocuments): ConditionDisplayEntry[] {
  const eff = resolveTokenActor(token, store);
  if (!eff) return [];
  const reg = store.query("condition-registry")[0]?.engine as ConditionRegistryEngine | undefined;
  const map = reg?.conditions ?? {};
  const out: ConditionDisplayEntry[] = [];
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
  /** The document to write conditions to: the linked actor, or the token itself (instanced). */
  doc: WireDocument;
  /** The JSON-pointer path to the conditions array on `doc` — either `/engine/conditions` or
   * `/embedded/actor/0/engine/conditions` per the linked-vs-instanced split above. */
  path: string;
  /** The current effective condition ids at `path`, read at resolution time. */
  conditions: string[];
}

/** Where a token's conditions live + the current set — see the `ConditionTarget` doc above for
 * the linked-vs-instanced write-path split.
 * @param token The token to resolve a condition write target for.
 * @param store The document store to resolve a linked actor against.
 * @returns The `{doc, path, conditions}` write target, or `null` for a raw/dangling token.
 * @example
 * ```ts
 * import { conditionTarget, type ReadableDocuments, type WireDocument } from "@shadowcat/core";
 *
 * declare const token: WireDocument;
 * declare const store: ReadableDocuments;
 * const target = conditionTarget(token, store);
 * target?.path; // "/engine/conditions" or "/embedded/actor/0/engine/conditions"
 * ```
 */
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
 * tokens, or between square and hex scenes. The box is READ from the server's resolved footprint
 * (`FootprintLookup.token`), never computed here: the geometry the client draws and the geometry
 * the movement gate collides with come from one definition, which lives server-side. When the
 * lookup states nothing — an unconfirmed optimistic token, a token no actor sizes, a REFUSED
 * size — the box falls back to the token document's own authored `w`/`h`, which the placement
 * path stamps from the scene's unit footprint. `(x,y)` is the token center. Pass a pre-resolved
 * `eff` (from a prior `resolveTokenActor` call) to avoid a second resolution; omit (or pass
 * `undefined`) to resolve internally. Pass `null` explicitly for a known actorless token to skip
 * resolution — `shape` is the only field `eff` decides. */
export interface TokenBox {
  /** Scene-pixel x coordinate of the token's center. */
  x: number;
  /** Scene-pixel y coordinate of the token's center. */
  y: number;
  /** Scene-pixel width of the footprint. */
  w: number;
  /** Scene-pixel height of the footprint. */
  h: number;
  /** The footprint shape used for hit-testing and the selection ring. */
  shape: "square" | "circle";
}

/** See the `TokenBox` doc above for the wire-extent-vs-authored-fallback resolution rule.
 * @param token The token to resolve a footprint for.
 * @param store The document store to resolve the actor against (for `shape`).
 * @param footprints The server's resolved footprints; supply `EMPTY_FOOTPRINTS` where none have
 * been received.
 * @param eff A pre-resolved `EffectiveActor` to reuse (skips a second `resolveTokenActor` call);
 * pass `null` for a known actorless token, or omit to resolve internally.
 * @returns The token's `{x, y, w, h, shape}` footprint in scene pixels.
 * @example
 * ```ts
 * import { resolveTokenBox, EMPTY_FOOTPRINTS, type ReadableDocuments, type WireDocument } from "@shadowcat/core";
 *
 * declare const token: WireDocument;
 * declare const store: ReadableDocuments;
 * resolveTokenBox(token, store, EMPTY_FOOTPRINTS); // { x, y, w, h, shape }
 * ```
 */
export function resolveTokenBox(token: WireDocument, store: ReadableDocuments, footprints: FootprintLookup, eff?: EffectiveActor | null): TokenBox {
  const eng = token.engine as TokenEngine | undefined;
  const actor = eff === undefined ? resolveTokenActor(token, store) : eff;
  const resolved = footprints.token(token.id);
  return {
    x: eng?.x ?? 0,
    y: eng?.y ?? 0,
    w: resolved?.w ?? eng?.w ?? 0,
    h: resolved?.h ?? eng?.h ?? 0,
    shape: actor?.shape ?? "square",
  };
}

/** The effective face names for a token's `faces`-union visual (the actor's own faces, with any
 * per-token `overrides.visual` union projected in) — the face-swap palette's option list. Reads
 * the same `resolveTokenActor` projection `resolveTokenVisual` reads, so the palette can never
 * diverge from what actually renders. Empty when the effective visual isn't `"faces"`.
 * @param token The token to resolve face names for.
 * @param store The document store to resolve the actor against.
 * @returns The effective face names, or `[]` when the effective visual isn't `"faces"`.
 * @example
 * ```ts
 * import { selectedFaceNamesFor, type ReadableDocuments, type WireDocument } from "@shadowcat/core";
 *
 * declare const token: WireDocument;
 * declare const store: ReadableDocuments;
 * selectedFaceNamesFor(token, store); // e.g. ["front", "back"]
 * ```
 */
export function selectedFaceNamesFor(token: WireDocument, store: ReadableDocuments): string[] {
  const eff = resolveTokenActor(token, store);
  return eff?.visual?.kind === "faces" ? Object.keys(eff.visual.faces) : [];
}

/** Resolve a `faces` visual to the active face's RenderVisual. Precedence: a valid manual
 * `token.engine.face` > the first `faceMap` entry whose condition id is in `conditions` (in
 * `conditions` array order — a v1 simplification, no severity ranking across simultaneously
 * active conditions) > `default` > the first key of `faces` (fail-closed continuation, never a
 * missing-visual null while any face exists). Returns null only when `faces` is empty. Not
 * exported (folded into `resolveTokenVisual`'s public surface).
 * @param visual The token's effective `"faces"`-kind `TokenVisual`.
 * @param manualFace The token's own `engine.face` selection, if any.
 * @param conditions The token's effective raw condition ids, in array order.
 * @returns The resolved `FaceVisual`, or `null` iff `visual.faces` is empty.
 * @example
 * ```
 * // internal helper; not part of the public API (see resolveTokenVisual for the public entry point)
 * declare const visual: Extract<TokenVisual, { kind: "faces" }>;
 * declare const token: WireDocument;
 * declare const actor: EffectiveActor | null;
 * resolveFace(visual, (token.engine as TokenEngine | undefined)?.face, actor?.conditions ?? []);
 * ```
 */
function resolveFace(
  visual: Extract<TokenVisual, { /** Narrows `TokenVisual` to its `"faces"` union member. */ kind: "faces" }>,
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

/** Structural validity of an `"animated"` `RenderVisual`: a finite positive `fps`, and either a
 * non-empty `frames` array (`"frames"` source) or positive-integer `rows`/`cols` (`"sheet"`
 * source). Not exported (folded into `resolveTokenVisual`'s public surface).
 * @param v The animated visual to validate.
 * @returns `true` iff `v` is structurally playable.
 * @example
 * ```
 * // internal helper; not part of the public API (see resolveTokenVisual for the public entry point)
 * isValidAnimated({ kind: "animated", source: { type: "frames", frames: ["a.png"] }, fps: 4, loop: true });
 * ```
 */
function isValidAnimated(v: Extract<RenderVisual, { /** Narrows `RenderVisual` to its `"animated"` union member. */ kind: "animated" }>): boolean {
  if (!Number.isFinite(v.fps) || v.fps <= 0) return false;
  if (v.source.type === "frames") return v.source.frames.length > 0;
  return Number.isInteger(v.source.rows) && v.source.rows > 0 && Number.isInteger(v.source.cols) && v.source.cols > 0;
}

/** The render boundary: resolves a token's `TokenVisual` (image, animated, or faces) down to a
 * plain `RenderVisual` (image or animated) — the only two kinds the render layer ever draws.
 * Fail-closed to `null` on any malformed/unknown shape; never throws. Pass a pre-resolved `eff`
 * to avoid a second `resolveTokenActor` call; omit to resolve internally.
 * @param token The token to resolve a visual for.
 * @param store The document store to resolve the actor against.
 * @param eff A pre-resolved `EffectiveActor` to reuse; pass `null` for a known actorless token,
 * or omit to resolve internally.
 * @returns The resolved `RenderVisual`, or `null` on any malformed/unresolvable shape.
 * @example
 * ```ts
 * import { resolveTokenVisual, type ReadableDocuments, type WireDocument } from "@shadowcat/core";
 *
 * declare const token: WireDocument;
 * declare const store: ReadableDocuments;
 * resolveTokenVisual(token, store); // { kind: "image", asset: "..." } | { kind: "animated", ... } | null
 * ```
 */
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
