import { z } from "zod";
import type { BarDisplayMode } from "@shadowcat/types";
import type { WireDocument } from "./wire";

/** One bar the server has already evaluated for this recipient. There is no client-side formula
 * evaluation: `current` and `max` are the server's numbers over the recipient's own redacted
 * view of the host document. */
export interface ResolvedBarView {
  /** The bar's label. */
  label: string;
  /** The evaluated filled amount. */
  current: number;
  /** The evaluated full amount. */
  max: number;
  /** Fill colour, a css `#rrggbb` string. */
  color: string;
  /** The bar's authored mode; `"hover"` bars are drawn only while the token is hovered or
   * selected, every other delivered mode draws continuously. */
  mode: BarDisplayMode;
}

/** One token's nameplate and bars exactly as the `"bars"` channel delivers them. */
export interface TokenBarsView {
  /** The name this recipient sees, or `null` when the recipient receives no name even though the
   * nameplate is delivered (the client draws its own fallback label). Meaningful only while
   * `nameplateMode` is non-null. */
  nameplate: string | null;
  /** The nameplate mode when the nameplate is delivered to this recipient, `null` when it is
   * not — a `null` draws no nameplate at all. */
  nameplateMode: BarDisplayMode | null;
  /** The bars delivered to this recipient, in authored order. */
  bars: ResolvedBarView[];
}

/** Read-only view of the resolved nameplates and bars the server has broadcast. Every query
 * answers `null` when the server has stated nothing for the token (not readable by this
 * recipient, nothing delivered, or no frame yet); a `null` draws nothing. */
export interface BarsLookup {
  /** The delivered nameplate and bars for one token.
   * @param tokenId - The token document id to look up.
   * @returns Its delivered entry, or `null` when the server has stated none. */
  token(tokenId: string): TokenBarsView | null;
}

/** The most bars one token or actor may author; the server rejects a longer list, so an editor
 * stops offering "add" at this count rather than letting a write fail. Mirrors the server's
 * `MAX_BAR_ATTRIBUTES`. */
export const MAX_BARS = 4;

/** The longest bar label, in characters; the server rejects a longer one. Mirrors the server's
 * `MAX_BAR_LABEL_CHARS`. */
export const MAX_BAR_LABEL_CHARS = 64;

/** Every display mode, in the order a mode select lists them: least to most visible. */
export const BAR_DISPLAY_MODES: readonly BarDisplayMode[] = ["none", "hover", "owner", "always"];

/** Wire shape of a display mode. */
const modeSchema = z.enum(["none", "hover", "owner", "always"]);

/** Wire shape of one resolved bar; a non-finite number fails the whole payload. */
const barSchema = z.object({
  label: z.string(),
  current: z.number().finite(),
  max: z.number().finite(),
  color: z.string(),
  mode: modeSchema,
});

/** Wire shape of one token's entry. */
const tokenSchema = z.object({
  nameplate: z.string().nullable(),
  nameplate_mode: modeSchema.nullable(),
  bars: z.array(barSchema),
});

/** Wire shape of the `"bars"` derived channel payload — the Zod mirror of the generated
 * `BarsPayload`. */
const payloadSchema = z.object({
  scenes: z.record(z.string(), z.object({ tokens: z.record(z.string(), tokenSchema) })),
});

/** A lookup that has nothing to say — every query is `null`. The state before the first
 * `"bars"` frame arrives, and the value a host with no subscription supplies. */
export const EMPTY_BARS: BarsLookup = { token: () => null };

/**
 * Parse a `"bars"` derived-channel payload into a lookup.
 *
 * A payload that does not validate yields {@link EMPTY_BARS} rather than a partial read: the
 * channel is the recipient's whole redacted entitlement, so a half-parsed set could draw some
 * values and silently drop others with no way for a caller to tell the two apart.
 * @param payload The raw `SceneDerived` payload for the `"bars"` channel.
 * @returns A lookup over the payload, or {@link EMPTY_BARS} when it does not validate.
 * @example
 * ```ts
 * import { parseBars } from "@shadowcat/core";
 *
 * declare const payload: unknown;
 * const bars = parseBars(payload);
 * bars.token("tok-1"); // { nameplate, nameplateMode, bars } | null
 * ```
 */
export function parseBars(payload: unknown): BarsLookup {
  const parsed = payloadSchema.safeParse(payload);
  if (!parsed.success) return EMPTY_BARS;
  const tokens = new Map<string, TokenBarsView>();
  for (const scene of Object.values(parsed.data.scenes)) {
    for (const [id, t] of Object.entries(scene.tokens)) {
      tokens.set(id, { nameplate: t.nameplate, nameplateMode: t.nameplate_mode, bars: t.bars });
    }
  }
  return { token: (id) => tokens.get(id) ?? null };
}

/**
 * The delivered nameplate and bars for one token — a pure read-through of the `"bars"` channel.
 * The server evaluated every number and made every visibility decision; this reads the result
 * and evaluates no formula.
 * @param token The token document to look up.
 * @param bars The server's resolved bars; supply {@link EMPTY_BARS} where none have arrived.
 * @returns The token's delivered entry, or `null` when the server stated none.
 * @example
 * ```ts
 * import { resolveTokenBarAttribute, EMPTY_BARS, type WireDocument } from "@shadowcat/core";
 *
 * declare const token: WireDocument;
 * resolveTokenBarAttribute(token, EMPTY_BARS); // null
 * ```
 */
export function resolveTokenBarAttribute(token: WireDocument, bars: BarsLookup): TokenBarsView | null {
  return bars.token(token.id);
}

/**
 * Whether a delivered nameplate or bar draws now. A `"hover"` value draws only while its token
 * is hovered or selected; every other delivered mode (`"owner"`, `"always"`) draws continuously.
 * The `"none"` mode is never delivered, so it never draws.
 * @param mode The value's delivered mode.
 * @param emphasized Whether the token is hovered or selected.
 * @returns `true` when the value draws.
 * @example
 * ```ts
 * import { barRevealed } from "@shadowcat/core";
 *
 * barRevealed("hover", false); // false
 * barRevealed("hover", true); // true
 * barRevealed("always", false); // true
 * ```
 */
export function barRevealed(mode: BarDisplayMode, emphasized: boolean): boolean {
  if (mode === "none") return false;
  return mode === "hover" ? emphasized : true;
}
