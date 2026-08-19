import { z } from "zod";

/** A token's resolved drawn extent in SCENE units, exactly as the server states it. There is no
 * client-side footprint formula: this value is read off the `"footprints"` derived channel, whose
 * every number the server computes from the one definition its movement gate collides with. */
export interface FootprintExtent {
  /** Bounding-box width in scene units. */
  w: number;
  /** Bounding-box height in scene units. */
  h: number;
}

/** Read-only view of the resolved footprints the server has broadcast. Every query answers
 * `null` when the server has stated nothing — a token it does not size (no actor, dangling link,
 * or unreadable by this recipient), a token whose size it REFUSES (degenerate or over-cap), or a
 * frame that has not arrived yet. A `null` grants nothing: the client's only uses of an extent
 * are drawing and picking, and the gate that grants passage is server-side. */
export interface FootprintLookup {
  /** The resolved extent for one token.
   * @param tokenId - The token document id to look up.
   * @returns Its resolved extent, or `null` when the server has stated none. */
  token(tokenId: string): FootprintExtent | null;
  /** The resolved extent of a 1x1 token in one scene's grid — what the placement path stamps
   * onto a token it creates.
   * @param sceneId - The scene document id to look up; `null`/`undefined` answers `null`.
   * @returns The scene's unit extent, or `null` when the server has stated none. */
  unit(sceneId: string | null | undefined): FootprintExtent | null;
}

/** Wire shape of one extent; non-finite or negative fails the whole payload rather than
 * rendering a garbled box. */
const extentSchema = z.object({ w: z.number().finite().nonnegative(), h: z.number().finite().nonnegative() });

/** Wire shape of the `"footprints"` derived channel payload — the Zod mirror of the generated
 * `FootprintsPayload`. */
const payloadSchema = z.object({
  scenes: z.array(
    z.object({
      scene: z.string(),
      unit: extentSchema,
      tokens: z.array(z.object({ token: z.string(), extent: extentSchema.nullable() })),
    }),
  ),
});

/** A lookup that has nothing to say — every query is `null`. The state before the first
 * `"footprints"` frame arrives, and the value a host with no subscription supplies. */
export const EMPTY_FOOTPRINTS: FootprintLookup = {
  token: () => null,
  unit: () => null,
};

/**
 * Parse a `"footprints"` derived-channel payload into a lookup.
 *
 * A payload that does not validate yields {@link EMPTY_FOOTPRINTS} rather than a partial read:
 * a half-parsed footprint set would mix authoritative extents with silently-dropped ones, and a
 * caller cannot tell those apart. Every consumer already handles `null` by falling back to the
 * token document's own authored extent, so the empty result is the safe one.
 * @param payload The raw `SceneDerived` payload for the `"footprints"` channel.
 * @returns A lookup over the payload, or {@link EMPTY_FOOTPRINTS} when it does not validate.
 * @example
 * ```ts
 * import { parseFootprints } from "@shadowcat/core";
 *
 * declare const payload: unknown;
 * const footprints = parseFootprints(payload);
 * footprints.token("tok-1"); // { w, h } | null
 * ```
 */
export function parseFootprints(payload: unknown): FootprintLookup {
  const parsed = payloadSchema.safeParse(payload);
  if (!parsed.success) return EMPTY_FOOTPRINTS;
  const tokens = new Map<string, FootprintExtent>();
  const units = new Map<string, FootprintExtent>();
  for (const s of parsed.data.scenes) {
    units.set(s.scene, s.unit);
    for (const t of s.tokens) {
      if (t.extent) tokens.set(t.token, t.extent);
    }
  }
  return {
    token: (id) => tokens.get(id) ?? null,
    unit: (id) => (id ? units.get(id) ?? null : null),
  };
}
