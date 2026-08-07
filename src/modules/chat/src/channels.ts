// Pure chat view model: which channel/GM view is active, filtering, sort, post
// target derivation. No Svelte/store dependency — ChatPanel wires this to reactive
// queries. `channel` is a purely client-side display label; the server enforces
// only `audience`, never `channel` (`build_message_doc`). "All" and per-channel
// views both read every message already present in this client's store
// regardless of audience; the GM view further filters on
// `audience.kind === "gm_only"`. This filtering is NOT a secrecy boundary — a
// player's store never held a `gm_only` message it wasn't a recipient of in the
// first place: redaction is per-recipient, filtering each connection's outgoing
// frame individually rather than in one shared pre-fan-out pass (`send_filtered`;
// the general rule: permissions are enforced server-side, per recipient).
import { parseMessageEngine, type ChatMessageEngine, type WireAudience, type WireDocument } from "@shadowcat/core";

/** Which slice of the store's `message` docs the panel currently renders — a display filter only, see `inView`. */
export type ChatView =
  | { /** Every message regardless of `channel`/`audience`. */ kind: "all" }
  | {
      /** Only messages whose `channel` matches `id`. */
      kind: "channel";
      /** The channel-registry key to filter on. */
      id: string;
    }
  | { /** Only messages whose `audience.kind === "gm_only"`. */ kind: "gm" };

/**
 * The `{channel, audience}` a message send from `view` should use: a channel
 * view posts to that channel as `public`; the GM view posts to the default
 * channel as `gm_only` (the server's `PermissionSet` mapping is what makes it
 * GM-only, not this label — see the module header); "All" posts to the
 * default channel as `public`, same as a plain channel post.
 * @param view The chat view currently active in the panel.
 * @returns The channel name and audience a send from `view` should carry.
 * @example
 * ```
 * // private module; not reachable as a workspace import — call shape:
 * postTarget({ kind: "gm" }); // → { channel: "general", audience: { kind: "gm_only" } }
 * ```
 */
export function postTarget(view: ChatView): {
  /** The channel a send from `view` should carry. */
  channel: string;
  /** The audience a send from `view` should carry. */
  audience: WireAudience;
} {
  if (view.kind === "channel") return { channel: view.id, audience: { kind: "public" } };
  if (view.kind === "gm") return { channel: "general", audience: { kind: "gm_only" } };
  return { channel: "general", audience: { kind: "public" } };
}

/**
 * Whether a message with parsed body `sys` belongs in `view`. "All" and a
 * per-channel view read every message regardless of `audience`; the GM view
 * is the one case that checks `audience` instead of `channel`. This is a
 * display filter over documents this client already has, not a security
 * check — the server, via the `PermissionSet` `build_message_doc` attaches
 * to the doc, is what keeps a `gm_only` or `whisper` message out of a
 * non-recipient's store to begin with.
 * @param view The chat view being rendered.
 * @param sys The message's parsed engine body.
 * @returns `true` if `sys` should be shown in `view`.
 * @example
 * ```
 * // private module; not reachable as a workspace import — call shape:
 * declare const sys: ChatMessageEngine;
 * inView({ kind: "gm" }, sys); // → sys.audience.kind === "gm_only"
 * ```
 */
export function inView(view: ChatView, sys: ChatMessageEngine): boolean {
  if (view.kind === "all") return true;
  if (view.kind === "gm") return sys.audience.kind === "gm_only";
  return sys.channel === view.id;
}

/**
 * Comparator: orders by envelope `created_at` ascending, then `id` ascending
 * as a tie-break (both server-set; neither changes on edit). `isAfter`
 * applies the identical ordering rule when testing a document
 * against a read-frontier marker.
 * @param a A document to compare.
 * @param b The other document to compare.
 * @returns Negative if `a` sorts before `b`, positive if after, 0 if equal.
 * @example
 * ```
 * // private module; not reachable as a workspace import — call shape:
 * declare const doc1: WireDocument;
 * declare const doc2: WireDocument;
 * [doc1, doc2].sort(byCreation);
 * ```
 */
export function byCreation(a: WireDocument, b: WireDocument): number {
  return a.created_at - b.created_at || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

export const RENDER_CAP = 200;

/**
 * Incremental view-membership + sort-order cache for one active chat view.
 * A message's `channel` and `audience` are frozen at creation and copied
 * verbatim by both revision paths, neither of which ever reassigns
 * `sys.channel`/`sys.audience` before re-serializing: `handle_edit_message`
 * and `handle_delete_message`. So an id's view membership and sorted
 * position never need recomputing once known — only a genuinely new
 * id costs a parse + insertion; an edit to a known id only refreshes its
 * stored reference. Reset (a fresh cache) on view change.
 */
export type ChatDerivationCache = {
  /** Ids currently in view, sorted by byCreation. */
  order: string[];
  /** Latest WireDocument reference per id; identity backs the re-parse skip. */
  refs: Map<string, WireDocument>;
  /** Cached view-membership per id (fixed for the id's lifetime in this view). */
  members: Map<string, boolean>;
};

/**
 * A fresh, empty `ChatDerivationCache` — call once per view (see
 * `ChatDerivationCache`'s doc comment) and replace on view change.
 * @returns A new cache with no cached ids.
 * @example
 * ```
 * // private module; not reachable as a workspace import — call shape:
 * let cache = createChatDerivationCache();
 * ```
 */
export function createChatDerivationCache(): ChatDerivationCache {
  return { order: [], refs: new Map(), members: new Map() };
}

// Counts every `parseMessageEngine` call `deriveVisibleDocs` makes below.
// Always incremented in production — there is no test-only gate on the
// counter itself. It is the two accessors below that are test-only: the
// "editing a message outside the current render window does not re-parse
// the full history" test is their sole caller in this package (verified: no
// non-test import of either exists here).
let parseCallCount = 0;
/**
 * Test-only accessor over the always-live `parseCallCount` counter.
 * @returns The number of `parseMessageEngine` calls made so far.
 * @example
 * ```
 * // private module; not reachable as a workspace import — call shape:
 * getParseCallCount(); // → 0 before any deriveVisibleDocs call
 * ```
 */
export function getParseCallCount(): number {
  return parseCallCount;
}
/**
 * Test-only accessor: resets the always-live `parseCallCount` counter to 0.
 * @example
 * ```
 * // private module; not reachable as a workspace import — call shape:
 * resetParseCallCount();
 * ```
 */
export function resetParseCallCount(): void {
  parseCallCount = 0;
}

/**
 * Binary-searches `cache.order` (already sorted by `byCreation`) for the
 * insertion point of `doc`, reading each candidate's cached `refs`
 * reference rather than re-parsing.
 * @param cache The cache whose `order`/`refs` back the search.
 * @param doc The document to find an insertion point for.
 * @returns The index in `cache.order` at which `doc.id` belongs.
 * @example
 * ```
 * // private function; not reachable as a workspace import — call shape:
 * declare const cache: ChatDerivationCache;
 * declare const doc: WireDocument;
 * insertionIndex(cache, doc);
 * ```
 */
function insertionIndex(cache: ChatDerivationCache, doc: WireDocument): number {
  let lo = 0;
  let hi = cache.order.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    const midDoc = cache.refs.get(cache.order[mid])!;
    if (byCreation(midDoc, doc) <= 0) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/**
 * Updates `cache` in place to reflect `allMessages` (every `message` doc the
 * store currently holds, unfiltered) for `view`, and returns the last `cap`
 * matching docs in sorted order. A doc whose reference matches the cached
 * reference is skipped entirely; a known id with a changed reference (an
 * edit) only refreshes `refs` — no re-parse, no reorder, since view
 * membership and sort position are both fixed at creation. Only a genuinely
 * new id costs a parse plus an O(log n) binary-search insertion; the full
 * history is never re-sorted.
 * @param cache The view's derivation cache, mutated in place.
 * @param allMessages Every `message` doc currently in the store, unfiltered.
 * @param view The active chat view to derive membership for.
 * @param cap The maximum number of matching docs to return.
 * @returns The last `cap` matching docs, sorted by `byCreation`.
 * @example
 * ```
 * // private module; not reachable as a workspace import — call shape:
 * declare const cache: ChatDerivationCache;
 * declare const allMessages: WireDocument[];
 * deriveVisibleDocs(cache, allMessages, { kind: "all" }, RENDER_CAP);
 * ```
 */
export function deriveVisibleDocs(cache: ChatDerivationCache, allMessages: WireDocument[], view: ChatView, cap: number): WireDocument[] {
  const seen = new Set<string>();
  for (const doc of allMessages) {
    seen.add(doc.id);
    if (cache.refs.get(doc.id) === doc) continue;
    cache.refs.set(doc.id, doc);
    let member = cache.members.get(doc.id);
    if (member === undefined) {
      parseCallCount++;
      const sys = parseMessageEngine(doc);
      member = sys !== null && inView(view, sys);
      cache.members.set(doc.id, member);
      if (member) cache.order.splice(insertionIndex(cache, doc), 0, doc.id);
    }
  }
  // A live message doc is never removed from this client's store by a
  // per-doc delete: `ops_target_message` rejects a client-authored
  // Create/Delete targeting a `message` doc_type at both ingress points
  // (guarding the WS `Intent` arm in `handle_socket` and the HTTP
  // `write_ops` mirror), and a message's `parent_id` is always `None`
  // (`build_message_doc`), so it can never be swept up as a descendant of
  // some other doc's cascading delete either — `handle_delete_message`
  // only ever produces a soft-tombstoning `Operation::Update`. A world
  // switch tears down this whole cache and store rather than removing ids
  // from a live one (a new `WorldSession`/`DocumentStore` per `App`'s
  // `enterWorld` call), so it is not a "removal" in the sense checked here
  // either. The scan below still runs correctly if
  // any of that ever changes; skipping it is only a fast path taken when
  // the store's message count didn't shrink. Reconciles against
  // `cache.refs`'s own key set, not just `cache.order` — a non-member id
  // (member === false) is cached in `refs`/`members` but never enters
  // `order`, so scanning only `order` would leave its entry behind forever
  // once the doc disappears.
  if (seen.size !== cache.refs.size) {
    for (const id of cache.refs.keys()) {
      if (!seen.has(id)) {
        const orderIdx = cache.order.indexOf(id);
        if (orderIdx !== -1) cache.order.splice(orderIdx, 1);
        cache.refs.delete(id);
        cache.members.delete(id);
      }
    }
  }
  const windowIds = cache.order.slice(Math.max(0, cache.order.length - cap));
  return windowIds.map((id) => cache.refs.get(id)!);
}

/** Overscan rows kept mounted beyond the estimated visible range, each side. */
export const VIRTUALIZE_OVERSCAN = 8;

/**
 * Ratio-based windowing: maps the scroll container's fractional scroll
 * position onto an index range within `totalCount`, using the OVERALL
 * average row density (`clientHeight / scrollHeight`) rather than dividing
 * by a single assumed row height — chat rows have variable
 * (text-wrap-dependent) height, so no one row height exists to divide by.
 * That makes `visibleCount` an ESTIMATE, not a guarantee: if the rows
 * currently in view happen to be shorter than the dataset's average, more
 * of them fit in `clientHeight` than the average-density estimate assumes,
 * and `overscan` (a fixed row count, not a pixel budget) is not guaranteed
 * to cover the gap. The one case that IS guaranteed to mount every row that
 * fits is the fallback taken when the container has no measured layout
 * (`clientHeight <= 0`) or content doesn't overflow it (`scrollHeight <=
 * clientHeight`) — that branch returns the full, unwindowed range, matching
 * unwindowed behavior when scroll metrics are unavailable (e.g. jsdom
 * without a layout engine).
 * @param scrollTop Current scroll offset of the messages container, in pixels.
 * @param clientHeight Visible height of the messages container, in pixels.
 * @param scrollHeight Total scrollable height of the messages container, in pixels.
 * @param totalCount Number of rows being windowed.
 * @param overscan Extra rows kept mounted beyond the estimated visible range, each side.
 * @returns A `{start, end}` index range (end-exclusive) into the `totalCount` rows.
 * @example
 * ```
 * // private module; not reachable as a workspace import — call shape:
 * declare const scrollTop: number;
 * declare const clientHeight: number;
 * declare const scrollHeight: number;
 * declare const totalCount: number;
 * computeVisibleWindow(scrollTop, clientHeight, scrollHeight, totalCount);
 * ```
 */
export function computeVisibleWindow(
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
  totalCount: number,
  overscan = VIRTUALIZE_OVERSCAN,
): {
  /** First index (inclusive) of the windowed range. */
  start: number;
  /** Last index (exclusive) of the windowed range. */
  end: number;
} {
  if (totalCount === 0) return { start: 0, end: 0 };
  if (clientHeight <= 0 || scrollHeight <= clientHeight) return { start: 0, end: totalCount };
  const fraction = Math.min(1, Math.max(0, scrollTop / (scrollHeight - clientHeight)));
  const visibleCount = Math.max(1, Math.ceil((clientHeight / scrollHeight) * totalCount));
  const firstVisible = Math.min(totalCount - 1, Math.floor(fraction * totalCount));
  const start = Math.max(0, firstVisible - overscan);
  const end = Math.min(totalCount, firstVisible + visibleCount + overscan);
  return { start, end };
}
