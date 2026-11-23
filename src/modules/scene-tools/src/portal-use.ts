import type { ReadableDocuments, RegionEngine, TokenEngine, WireDocument } from "@shadowcat/core";
import { topRegionAt } from "./hit-test";

/** Whether a region has a trigger that offers a portal: a `portal_choice` effect on any event
 * but `exit`. Mirrors the server's `offers_portal` predicate, which decides which stored
 * region membership `OpenPortal` re-offers from.
 * @param region The region document to test.
 * @returns `true` when the region is enabled and carries an offering trigger.
 * @example
 * ```
 * declare const region: WireDocument;
 * offersPortal(region);
 * ```
 */
export function offersPortal(region: WireDocument): boolean {
  const engine = region.engine as RegionEngine | undefined;
  if (!engine || !engine.enabled) return false;
  return engine.triggers.some((t) => t.effect.type === "portal_choice" && t.on !== "exit");
}

/** The ids among `tokenIds` whose centre stands inside a region that offers a portal, so the
 * rail can show a "Use portal" action for them. A display aid only: the server re-checks the
 * token's stored membership and the caller's ownership before it re-offers, and a region the
 * caller cannot see never reaches this document store.
 * @param tokenIds The candidate token ids (typically the token selection).
 * @param store The document store the tokens and regions are read from.
 * @returns The subset of `tokenIds` standing in a portal region, in input order.
 * @example
 * ```
 * declare const store: ReadableDocuments;
 * portalTokens(["token-1"], store); // ["token-1"] while it stands in a portal region
 * ```
 */
export function portalTokens(tokenIds: Iterable<string>, store: ReadableDocuments): string[] {
  const regions = store.query("region").filter(offersPortal);
  if (regions.length === 0) return [];
  const out: string[] = [];
  for (const id of tokenIds) {
    const token = store.get(id);
    const engine = token?.engine as TokenEngine | undefined;
    if (!token || token.doc_type !== "token" || !engine) continue;
    const here = regions.filter((r) => r.parent_id === token.parent_id);
    const centre = { x: engine.x, y: engine.y };
    if (here.some((r) => topRegionAt([r], centre, store) !== null)) out.push(id);
  }
  return out;
}
