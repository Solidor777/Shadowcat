import { pointerToken, type ActorEngine, type BarAttribute, type BarsLookup, type ReadableDocuments, type TokenEngine, type WireDocument } from "@shadowcat/core";
import { isBareLeafReference } from "@shadowcat/formula";

/** Where a quick-edit nudge of one bar writes: the `system` leaf its `current` formula names. */
export interface QuickBarTarget {
  /** The document holding the leaf: the linked actor, or the token itself when it embeds the
   * actor copy (the same split `conditionTarget` makes). */
  doc: WireDocument;
  /** JSON pointer to the leaf on `doc`, e.g. `/system/hp/value`. */
  path: string;
}

/** One bar row of the quick-edit section: a bar the server delivered to this viewer. */
export interface QuickBar {
  /** The bar's label. */
  label: string;
  /** The server-evaluated current value, also the optimistic-concurrency pre-image of a write. */
  current: number;
  /** The server-evaluated maximum. */
  max: number;
  /** The leaf a nudge writes, or `null` when `current` is not a bare reference (a computed
   * value has no single leaf to write) or no authored bar could be matched to the row. */
  target: QuickBarTarget | null;
}

/**
 * The quick-edit bar rows for one token: exactly the bars the `"bars"` channel delivered to this
 * viewer, each paired with the write target its authored `current` formula names.
 *
 * The delivered list is the authored list filtered by what this viewer may see, still in authored
 * order, so each delivered bar is matched to the next authored bar with the same label, colour
 * and mode. A wrong match for two look-alike bars writes with an `old` that differs from the
 * stored leaf, which the server's optimistic-concurrency check rejects.
 * @param token The token document.
 * @param store The document store a linked actor is read from.
 * @param bars The server's resolved bars.
 * @returns The rows in delivered order; empty when nothing is delivered or the token resolves no
 * actor (raw token, dangling link).
 * @example
 * ```ts
 * import { EMPTY_BARS, type ReadableDocuments, type WireDocument } from "@shadowcat/core";
 * import { resolveQuickBars } from "./barQuickEdit";
 *
 * declare const token: WireDocument;
 * declare const store: ReadableDocuments;
 * resolveQuickBars(token, store, EMPTY_BARS); // []
 * ```
 */
export function resolveQuickBars(token: WireDocument, store: ReadableDocuments, bars: BarsLookup): QuickBar[] {
  const delivered = bars.token(token.id)?.bars ?? [];
  if (delivered.length === 0) return [];
  const eng = token.engine as TokenEngine | undefined;
  let host: WireDocument;
  let prefix: string;
  let authored: BarAttribute[];
  if (eng?.actor_id) {
    const actor = store.get(eng.actor_id);
    if (!actor) return [];
    host = actor;
    prefix = "/system";
    authored = eng.overrides?.bars ?? (actor.engine as ActorEngine).bars ?? [];
  } else {
    const embedded = token.embedded?.actor?.[0];
    if (!embedded) return [];
    host = token;
    prefix = "/embedded/actor/0/system";
    authored = (embedded.engine as ActorEngine).bars ?? [];
  }
  let next = 0;
  return delivered.map((d) => {
    let match: BarAttribute | null = null;
    for (let i = next; i < authored.length; i += 1) {
      const a = authored[i];
      if (a.label === d.label && a.color === d.color && a.mode === d.mode) {
        match = a;
        next = i + 1;
        break;
      }
    }
    const leaf = match === null ? null : isBareLeafReference(match.current);
    return {
      label: d.label,
      current: d.current,
      max: d.max,
      target: leaf === null ? null : { doc: host, path: `${prefix}/${leaf.split(".").map(pointerToken).join("/")}` },
    };
  });
}
