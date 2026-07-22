// Pure chat view model: which channel/GM view is active, filtering, sort, post
// target derivation. No Svelte/store dependency — ChatPanel wires this to reactive
// queries. `channel` itself is a purely client-side label (chat skill: the server
// enforces only `audience`, never `channel`); "All" and per-channel views both read
// EVERY message regardless of audience, while the GM view filters on `audience`.
import { parseMessageEngine, type ChatMessageEngine, type WireAudience, type WireDocument } from "@shadowcat/core";

export type ChatView = { kind: "all" } | { kind: "channel"; id: string } | { kind: "gm" };

/** Post target for a view: All → the default channel; GM → gm_only audience. */
export function postTarget(view: ChatView): { channel: string; audience: WireAudience } {
  if (view.kind === "channel") return { channel: view.id, audience: { kind: "public" } };
  if (view.kind === "gm") return { channel: "general", audience: { kind: "gm_only" } };
  return { channel: "general", audience: { kind: "public" } };
}

export function inView(view: ChatView, sys: ChatMessageEngine): boolean {
  if (view.kind === "all") return true;
  if (view.kind === "gm") return sys.audience.kind === "gm_only";
  return sys.channel === view.id;
}

/** Sort by envelope created_at then id (server-set; stable under edits). */
export function byCreation(a: WireDocument, b: WireDocument): number {
  return a.created_at - b.created_at || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

export const RENDER_CAP = 200;

/**
 * Incremental view-membership + sort-order cache for one active chat view.
 * A message's `channel`/`audience` are frozen at creation (chat skill: both
 * are always copied verbatim from the stored doc on edit, never re-derived),
 * so an id's view membership and sorted position never need recomputing once
 * known — only a genuinely new id costs a parse + insertion; an edit to a
 * known id only refreshes its stored reference. Reset (a fresh cache) on
 * view change.
 */
export type ChatDerivationCache = {
  /** Ids currently in view, sorted by byCreation. */
  order: string[];
  /** Latest WireDocument reference per id; identity backs the re-parse skip. */
  refs: Map<string, WireDocument>;
  /** Cached view-membership per id (fixed for the id's lifetime in this view). */
  members: Map<string, boolean>;
};

export function createChatDerivationCache(): ChatDerivationCache {
  return { order: [], refs: new Map(), members: new Map() };
}

let parseCallCount = 0;
/** Test-only instrumentation: counts parseMessageEngine calls made by deriveVisibleDocs. */
export function getParseCallCount(): number {
  return parseCallCount;
}
export function resetParseCallCount(): void {
  parseCallCount = 0;
}

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
  // Removal never happens in practice (messages are soft-tombstoned in
  // place, never hard-deleted from the store); skip the scan unless the
  // store's message count actually shrank.
  if (seen.size !== cache.refs.size) {
    for (let i = cache.order.length - 1; i >= 0; i--) {
      const id = cache.order[i];
      if (!seen.has(id)) {
        cache.order.splice(i, 1);
        cache.refs.delete(id);
        cache.members.delete(id);
      }
    }
  }
  const windowIds = cache.order.slice(Math.max(0, cache.order.length - cap));
  return windowIds.map((id) => cache.refs.get(id)!);
}

/** Overscan rows kept mounted beyond the measured visible range, each side. */
export const VIRTUALIZE_OVERSCAN = 8;

/**
 * Ratio-based windowing: maps the scroll container's fractional scroll
 * position onto an index range within `totalCount`, rather than dividing by
 * an assumed fixed row height — chat rows have variable (text-wrap-dependent)
 * height, so a pixel/row-height approach would drift from the real layout.
 * Falls back to the full range when the container has no measured layout
 * (`clientHeight <= 0`) or content doesn't overflow it — never mounts fewer
 * rows than fit, and matches unwindowed behavior when scroll metrics are
 * unavailable (e.g. jsdom without a layout engine).
 */
export function computeVisibleWindow(
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
  totalCount: number,
  overscan = VIRTUALIZE_OVERSCAN,
): { start: number; end: number } {
  if (totalCount === 0) return { start: 0, end: 0 };
  if (clientHeight <= 0 || scrollHeight <= clientHeight) return { start: 0, end: totalCount };
  const fraction = Math.min(1, Math.max(0, scrollTop / (scrollHeight - clientHeight)));
  const visibleCount = Math.max(1, Math.ceil((clientHeight / scrollHeight) * totalCount));
  const firstVisible = Math.min(totalCount - 1, Math.floor(fraction * totalCount));
  const start = Math.max(0, firstVisible - overscan);
  const end = Math.min(totalCount, firstVisible + visibleCount + overscan);
  return { start, end };
}
